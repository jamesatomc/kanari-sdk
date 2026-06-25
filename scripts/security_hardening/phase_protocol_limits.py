from .common import read, write


def apply():
    path = "crates/kanari-core/src/engine/mempool.rs"
    text = read(path).replace(
        "const MAX_TRANSACTION_BYTES: usize = 256 * 1024;",
        "const MAX_TRANSACTION_BYTES: usize = 64 * 1024;",
        1,
    )
    write(path, text)

    for path in (
        "crates/kanari-core/src/engine/produce_dag_vertex.rs",
        "crates/kanari-core/src/engine/apply_checkpoint.rs",
    ):
        text = read(path)
        text = text.replace("8 * 1024 * 1024", "128 * 1024")
        write(path, text)
