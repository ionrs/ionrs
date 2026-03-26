use zed_extension_api::{self as zed, LanguageServerId, Result};

struct IonExtension;

impl zed::Extension for IonExtension {
    fn new() -> Self {
        IonExtension
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let path = worktree
            .which("ion-lsp")
            .ok_or_else(|| "ion-lsp not found on PATH. Install with: cargo install --path ion-lsp".to_string())?;

        Ok(zed::Command {
            command: path,
            args: vec![],
            env: vec![],
        })
    }
}

zed::register_extension!(IonExtension);
