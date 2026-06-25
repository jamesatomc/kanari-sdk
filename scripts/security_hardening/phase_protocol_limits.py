from .common import read, write


def apply():
    for path in (
        "crates/kanari-core/src/engine/produce_dag_vertex.rs",
        "crates/kanari-core/src/engine/apply_checkpoint.rs",
    ):
        text = read(path)
        text = text.replace("8 * 1024 * 1024", "512 * 1024")
        write(path, text)
