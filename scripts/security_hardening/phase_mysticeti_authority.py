from .common import read, write


def apply():
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    text = text.replace(
        "    fn new(authority_count: usize) -> Result<Self> {\n        let authority_count = authority_count.max(1);",
        "    fn new(local_authority: usize, authority_count: usize) -> Result<Self> {\n        let authority_count = authority_count.max(1);\n        anyhow::ensure!(local_authority < authority_count, \"local Mysticeti authority is outside the committee\");\n        let local_authority = MysticetiAuthority::from(local_authority);",
        1,
    )
    text = text.replace(
        "            MysticetiAuthority::default(),\n            metrics.clone(),",
        "            local_authority,\n            metrics.clone(),",
        1,
    )
    text = text.replace(
        "            MysticetiAuthority::default(),\n            committee.clone(),",
        "            local_authority,\n            committee.clone(),",
        1,
    )
    text = text.replace(
        "        let mysticeti = MysticetiBackend::new(authorities.len())?;",
        "        let local_index = authorities\n            .iter()\n            .position(|authority| authority == &authority_id)\n            .ok_or_else(|| anyhow::anyhow!(\"local authority is missing from the committee\"))?;\n        let mysticeti = MysticetiBackend::new(local_index, authorities.len())?;",
        1,
    )
    write(path, text)
