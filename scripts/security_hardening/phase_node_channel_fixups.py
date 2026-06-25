from .common import read, write


def apply():
    path = "crates/kanari-node/src/app.rs"
    text = read(path)
    text = text.replace("UnboundedSender<P2PMessage>", "Sender<P2PMessage>")
    text = text.replace("match network_tx.send(msg)", "match network_tx.try_send(msg)", 1)
    write(path, text)

    path = "crates/kanari-node/Cargo.toml"
    text = read(path)
    if "bcs = { workspace = true }" not in text:
        text = text.replace("bincode = { workspace = true }", "bincode = { workspace = true }\nbcs = { workspace = true }", 1)
    write(path, text)

    for path in ("crates/kanari-node/tests/unit/test_support.rs", "crates/kanari-node/tests/unit/sync_tests.rs"):
        write(path, read(path).replace("mpsc::unbounded_channel()", "mpsc::channel(32)"))
