extern crate bindings_wasm;
extern crate wasm_bindgen_test;
use bindings_wasm::*;
use prost::Message;
use wasm_bindgen_test::*;
use xmtp_cryptography::signature::RecoverableSignature;
use xmtp_proto::xmtp::message_api::v1::Envelope;

// Only run these tests in a browser.
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
pub fn test_client_construction() {
    let client = WasmXmtpClient::new();
    // TODO: assert things about the client once it does anything.
    assert!(matches!(client, Ok(_)));
}

#[wasm_bindgen_test]
pub fn test_xmtp_proto_in_wasm() {
    // Use some basic stuff from `xmtp_proto` to test that it works in Wasm.
    // TODO: cut once we have a full wasm client to test

    let env1 = Envelope {
        timestamp_ns: 12345,
        content_topic: "abc123".to_string(),
        message: vec![65],
    };
    let mut buf = Vec::new();
    buf.reserve(env1.encoded_len());
    env1.encode(&mut buf).unwrap();
    let env2 = Envelope::decode(buf.as_slice()).unwrap();
    assert_eq!(12345, env2.timestamp_ns);
    assert_eq!("abc123".to_string(), env2.content_topic);
}

#[wasm_bindgen_test]
pub fn test_xmtp_cryptography_in_wasm() {
    // (cribbed from `xmtp_cryptography`) tests that we can use it in Wasm.
    // TODO: cut once we have a full wasm client to test

    // This test was generated using Etherscans Signature tool: https://etherscan.io/verifySig/18959
    let addr = "0x1B2a516d691aBb8f08a75B2C73c95c62A1632431";
    let msg = "TestVector1";
    let sig_hash = "19d6bec562518e365d07ba3cce26d08a5fffa2cbb1e7fe03c1f2d6a722fd3a5e544097b91f8f8cd11d43b032659f30529139ab1a9ecb6c81ed4a762179e87db81c";

    let addr_alt = addr.strip_prefix("0x").unwrap();
    let addr_bad = &addr.replacen('b', "c", 1);
    let sig_bytes = hex::decode(sig_hash).unwrap();
    let sig = RecoverableSignature::Eip191Signature(sig_bytes);
    let msg_bad = "Testvector1";

    let recovered_addr = sig.recover_address(msg).unwrap();
    assert_eq!(recovered_addr, addr.to_lowercase());

    assert!(sig.verify_signature(addr, msg).is_ok());
    assert!(sig.verify_signature(addr_alt, msg).is_ok());
    assert!(sig.verify_signature(addr_bad, msg).is_err());
    assert!(sig.verify_signature(addr, msg_bad).is_err());
}
