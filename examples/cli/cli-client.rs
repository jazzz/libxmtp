/*
XLI is a Commandline client using XMTPv3.
*/

extern crate ethers;
extern crate log;
extern crate xmtp;

use clap::{Parser, Subcommand};
use ethers_core::types::H160;
use log::{error, info};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use url::ParseError;
use walletconnect::client::{CallError, ConnectorError, SessionError};
use walletconnect::{qr, Client as WcClient, Metadata};
use xmtp::builder::{AccountStrategy, ClientBuilderError};
use xmtp::client::ClientError;
use xmtp::conversation::{ConversationError, ListMessagesOptions, SecretConversation};
use xmtp::conversations::Conversations;
use xmtp::storage::{
    now, EncryptedMessageStore, EncryptionKey, MessageState, StorageError, StorageOption,
};
use xmtp::types::networking::XmtpApiClient;
use xmtp::InboxOwner;
use xmtp_cryptography::signature::{h160addr_to_string, RecoverableSignature, SignatureError};
use xmtp_cryptography::utils::{rng, seeded_rng, LocalWallet};
use xmtp_networking::grpc_api_helper::Client as ApiClient;
type Client = xmtp::client::Client<ApiClient>;
type ClientBuilder = xmtp::builder::ClientBuilder<ApiClient, Wallet>;

/// A fictional versioning CLI
#[derive(Debug, Parser)] // requires `derive` feature
#[command(name = "xli")]
#[command(about = "A lightweight XMTP console client", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Sets a custom config file
    #[arg(long, value_name = "FILE", global = true)]
    db: Option<PathBuf>,
    #[clap(long, default_value_t = false)]
    local: bool,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Register Account on XMTP Network
    Register {
        #[clap(long = "seed", default_value_t = 0)]
        wallet_seed: u64,
    },
    // List conversations on the registered wallet
    ListConversations {},
    /// Information about the account that owns the DB
    Info {},
    /// Send Message
    Send {
        #[arg(value_name = "ADDR")]
        addr: String,
        #[arg(value_name = "Message")]
        msg: String,
    },
    Recv {},
    ListContacts {},
    Clear {},
}

#[derive(Debug, Error)]
enum CliError {
    #[error("Walletconnect connection failed")]
    WcConnection(#[from] ConnectorError),
    #[error("Walletconnect session failed")]
    WcSession(#[from] SessionError),
    #[error("Walletconnect parse failed")]
    WcParse(#[from] ParseError),
    #[error("Walletconnect call failed")]
    WcCall(#[from] CallError),
    #[error("signature failed to generate")]
    Signature(#[from] SignatureError),
    #[error("stored error occured")]
    MessageStore(#[from] StorageError),
    #[error("client error")]
    ClientError(#[from] ClientError),
    #[error("clientbuilder error")]
    ClientBuilder(#[from] ClientBuilderError),
    #[error("ConversationError: {0}")]
    ConversationError(#[from] ConversationError),
    #[error("generic:{0}")]
    Generic(String),
}

impl From<String> for CliError {
    fn from(value: String) -> Self {
        Self::Generic(value)
    }
}

impl From<&str> for CliError {
    fn from(value: &str) -> Self {
        Self::Generic(value.to_string())
    }
}
/// This is an abstraction which allows the CLI to choose between different wallet types.
enum Wallet {
    WalletConnectWallet(WalletConnectWallet),
    LocalWallet(LocalWallet),
}

impl InboxOwner for Wallet {
    fn get_address(&self) -> String {
        match self {
            Wallet::WalletConnectWallet(w) => w.get_address(),
            Wallet::LocalWallet(w) => w.get_address(),
        }
    }

    fn sign(&self, text: &str) -> Result<RecoverableSignature, SignatureError> {
        match self {
            Wallet::WalletConnectWallet(w) => w.sign(text),
            Wallet::LocalWallet(w) => w.sign(text),
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();
    info!("Starting CLI Client....");

    let cli = Cli::parse();

    if let Commands::Register { wallet_seed } = &cli.command {
        info!("Register");
        if let Err(e) = register(&cli, true, wallet_seed).await {
            error!("Registration failed: {:?}", e)
        }
        return;
    }

    match &cli.command {
        #[allow(unused_variables)]
        Commands::Register { wallet_seed } => {
            unreachable!()
        }
        Commands::Info {} => {
            info!("Info");
            let client = create_client(&cli, AccountStrategy::CachedOnly("nil".into()))
                .await
                .unwrap();
            info!("Address is: {}", client.wallet_address());
            info!("Installation_id: {}", client.installation_id());
        }
        Commands::ListConversations {} => {
            info!("List Conversations");
            let client = create_client(&cli, AccountStrategy::CachedOnly("nil".into()))
                .await
                .unwrap();

            recv(&client).await.unwrap();
            let convo_list = Conversations::list(&client, true).await.unwrap();

            for (index, convo) in convo_list.iter().enumerate() {
                info!(
                    "====== [{}] Convo with {} ======{}{}",
                    index,
                    convo.peer_address(),
                    "\n",
                    format_messages(convo).await.unwrap()
                );
            }
        }
        Commands::Send { addr, msg } => {
            info!("Send");
            let client = create_client(&cli, AccountStrategy::CachedOnly("nil".into()))
                .await
                .unwrap();
            info!("Address is: {}", client.wallet_address());
            send(client, addr, msg).await.unwrap();
        }
        Commands::Recv {} => {
            info!("Recv");
            let client = create_client(&cli, AccountStrategy::CachedOnly("nil".into()))
                .await
                .unwrap();
            info!("Address is: {}", client.wallet_address());
            recv(&client).await.unwrap();
        }
        Commands::ListContacts {} => {
            let client = create_client(&cli, AccountStrategy::CachedOnly("nil".into()))
                .await
                .unwrap();

            let contacts = client.get_contacts(&client.wallet_address()).await.unwrap();
            for (index, contact) in contacts.iter().enumerate() {
                info!(" [{}]  Contact: {:?}", index, contact.installation_id());
            }
        }
        Commands::Clear {} => {
            fs::remove_file(&cli.db.unwrap()).unwrap();
        }
    }
}

async fn create_client(cli: &Cli, account: AccountStrategy<Wallet>) -> Result<Client, CliError> {
    let msg_store = get_encrypted_store(&cli.db).unwrap();
    let mut builder = ClientBuilder::new(account).store(msg_store);

    if cli.local {
        builder = builder
            .network(xmtp::Network::Local("http://localhost:5556"))
            .api_client(
                ApiClient::create("http://localhost:5556".into(), false)
                    .await
                    .unwrap(),
            );
    } else {
        builder = builder.network(xmtp::Network::Dev).api_client(
            ApiClient::create("https://dev.xmtp.network:5556".into(), true)
                .await
                .unwrap(),
        );
    }

    builder.build().map_err(CliError::ClientBuilder)
}

async fn register(cli: &Cli, use_local_db: bool, wallet_seed: &u64) -> Result<(), CliError> {
    let w = if use_local_db {
        if wallet_seed == &0 {
            Wallet::LocalWallet(LocalWallet::new(&mut rng()))
        } else {
            Wallet::LocalWallet(LocalWallet::new(&mut seeded_rng(*wallet_seed)))
        }
    } else {
        // Deprecated - WalletConnect V1 is no longer supported and WalletConnect V2
        // has no rust clients (yet)
        Wallet::WalletConnectWallet(WalletConnectWallet::create().await?)
    };

    let mut client = create_client(cli, AccountStrategy::CreateIfNotFound(w)).await?;
    info!("Address is: {}", client.wallet_address());

    if let Err(e) = client.init().await {
        error!("Initialization Failed: {}", e.to_string());
        panic!("Could not init");
    };

    Ok(())
}

async fn send(client: Client, addr: &str, msg: &String) -> Result<(), CliError> {
    let conversation = SecretConversation::new(&client, addr.to_string()).unwrap();
    conversation.initialize().await.unwrap();
    conversation.send_text(msg).await.unwrap();
    info!("Message successfully sent");

    Ok(())
}

async fn recv(client: &Client) -> Result<(), CliError> {
    Conversations::receive(client)?;
    Ok(())
}

async fn format_messages<'c, A: XmtpApiClient>(
    convo: &SecretConversation<'c, A>,
) -> Result<String, CliError> {
    let mut output: Vec<String> = vec![];
    let opts = ListMessagesOptions::default();

    for msg in convo.list_messages(&opts).await? {
        let contents = msg.get_text().map_err(|e| e.to_string())?;
        let is_inbound = msg.state == MessageState::Received as i32;
        let direction = if is_inbound {
            String::from("    -------->")
        } else {
            String::from("<--------    ")
        };

        let msg_line = format!(
            "[{:>15} ]    {}       {}",
            pretty_delta(now() as u64, msg.created_at as u64),
            direction,
            contents
        );
        output.push(msg_line);
    }
    output.reverse();

    Ok(output.join("\n"))
}

fn static_enc_key() -> EncryptionKey {
    [2u8; 32]
}

fn get_encrypted_store(db: &Option<PathBuf>) -> Result<EncryptedMessageStore, CliError> {
    let store = match db {
        Some(path) => {
            let s = path.as_path().to_string_lossy().to_string();
            info!("Using persistent storage: {} ", s);
            EncryptedMessageStore::new_unencrypted(StorageOption::Persistent(s))
        }

        None => {
            info!("Using ephemeral store");
            EncryptedMessageStore::new(StorageOption::Ephemeral, static_enc_key())
        }
    };

    store.map_err(|e| e.into())
}

fn pretty_delta(now: u64, then: u64) -> String {
    let f = timeago::Formatter::new();
    f.convert(Duration::from_nanos(now - then))
}

/// This wraps a Walletconnect::client into a struct which could be used in the xmtp::client.
struct WalletConnectWallet {
    addr: String,
    client: WcClient,
}

impl WalletConnectWallet {
    pub async fn create() -> Result<Self, CliError> {
        let client = WcClient::new(
            "examples-cli",
            Metadata {
                description: "XMTP CLI.".into(),
                url: "https://github.com/xmtp/libxmtp".parse()?,
                icons: vec![
                    "https://gateway.ipfs.io/ipfs/QmaSZuaXfNUwhF7khaRxCwbhohBhRosVX1ZcGzmtcWnqav"
                        .parse()?,
                ],
                name: "XMTP CLI".into(),
            },
        )?;

        let (accounts, _) = client.ensure_session(qr::print_with_url).await?;

        for account in &accounts {
            info!(" Connected account: {:?}", account);
        }

        Ok(Self {
            addr: h160addr_to_string(H160::from_slice(accounts[0].as_bytes())),
            client,
        })
    }
}

impl InboxOwner for WalletConnectWallet {
    fn get_address(&self) -> String {
        self.addr.clone()
    }

    fn sign(
        &self,
        text: &str,
    ) -> Result<
        xmtp_cryptography::signature::RecoverableSignature,
        xmtp_cryptography::signature::SignatureError,
    > {
        let sig = futures::executor::block_on(async { self.client.personal_sign(&[text]).await })
            .map_err(|e| SignatureError::ThirdPartyError(e.to_string()))?;

        Ok(RecoverableSignature::Eip191Signature(sig.to_vec()))
    }
}
