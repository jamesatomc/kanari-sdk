from .common import read, write


def apply():
    path = "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs"
    text = read(path)
    marker = '''    pub fn reload_vm_cache(&self) -> Result<()> {
        let new_vm = MoveVM::new(self.all_natives.as_ref().clone())'''
    replacement = '''    pub fn reload_vm_cache(&self) -> Result<()> {
        let modules: HashSet<ModuleId> = self.state.get_all_module_ids()?.into_iter().collect();
        *self
            .published_modules
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = modules;
        let new_vm = MoveVM::new(self.all_natives.as_ref().clone())'''
    if marker not in text:
        raise RuntimeError("runtime reload marker not found")
    write(path, text.replace(marker, replacement, 1))
