use zed_extension_api::{self as zed, settings::LspSettings, LanguageServerId, Result};

const SERVER_ID: &str = "ion-lsp";

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
        if let Some(command) = configured_command(worktree)? {
            return Ok(command);
        }

        discovered_command(worktree).ok_or_else(|| missing_server_message(worktree))
    }
}

zed::register_extension!(IonExtension);

fn configured_command(worktree: &zed::Worktree) -> Result<Option<zed::Command>> {
    let settings = LspSettings::for_worktree(SERVER_ID, worktree)?;
    let Some(binary) = settings.binary else {
        return Ok(None);
    };

    let command = binary
        .path
        .filter(|path| !path.trim().is_empty())
        .unwrap_or_else(|| SERVER_ID.to_string());
    let args = binary.arguments.unwrap_or_default();
    let mut env = binary
        .env
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    env.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(Some(zed::Command { command, args, env }))
}

fn discovered_command(worktree: &zed::Worktree) -> Option<zed::Command> {
    if let Some(path) = worktree.which(SERVER_ID) {
        return Some(zed::Command {
            command: path,
            args: vec![],
            env: vec![],
        });
    }

    if let Some(command) = wsl_command(worktree) {
        return Some(command);
    }

    cargo_home_command(worktree)
}

fn wsl_command(worktree: &zed::Worktree) -> Option<zed::Command> {
    let wsl = worktree
        .which("wsl.exe")
        .or_else(|| worktree.which("wsl"))?;
    let wsl_path = parse_wsl_project_path(&worktree.root_path())?;

    Some(zed::Command {
        command: wsl,
        args: vec![
            "-d".to_string(),
            wsl_path.distro,
            "--cd".to_string(),
            wsl_path.linux_path,
            SERVER_ID.to_string(),
        ],
        env: vec![],
    })
}

fn cargo_home_command(worktree: &zed::Worktree) -> Option<zed::Command> {
    let env = worktree.shell_env();
    let home = env
        .iter()
        .find_map(|(key, value)| (key == "HOME").then_some(value))?;
    let shell = worktree.which("sh")?;
    let candidate = format!("{home}/.cargo/bin/{SERVER_ID}");
    let probe = zed::process::Command::new(shell)
        .args(["-c", "test -x \"$1\"", "sh", &candidate])
        .output()
        .ok()?;

    (probe.status == Some(0)).then_some(zed::Command {
        command: candidate,
        args: vec![],
        env: vec![],
    })
}

fn missing_server_message(worktree: &zed::Worktree) -> String {
    let mut message =
        format!("{SERVER_ID} not found on PATH. Install it with: cargo install --path ion-lsp");

    if parse_wsl_project_path(&worktree.root_path()).is_some() {
        message.push_str(
            ", or configure Zed with: { \"lsp\": { \"ion-lsp\": { \"binary\": { \"path\": \"wsl.exe\", \"arguments\": [\"ion-lsp\"] } } } }",
        );
    }

    message
}

#[derive(Debug, PartialEq, Eq)]
struct WslProjectPath {
    distro: String,
    linux_path: String,
}

fn parse_wsl_project_path(path: &str) -> Option<WslProjectPath> {
    let normalized = path.replace('/', "\\");
    let wsl_localhost_prefix = "\\\\wsl.localhost\\";
    let wsl_dollar_prefix = "\\\\wsl$\\";
    let prefix_len = if normalized
        .to_ascii_lowercase()
        .starts_with(wsl_localhost_prefix)
    {
        wsl_localhost_prefix.len()
    } else if normalized
        .to_ascii_lowercase()
        .starts_with(wsl_dollar_prefix)
    {
        wsl_dollar_prefix.len()
    } else {
        return None;
    };

    let rest = &normalized[prefix_len..];
    let first_separator = rest.find('\\')?;
    if first_separator == 0 {
        return None;
    }

    let distro = rest[..first_separator].to_string();
    let distro_path = &rest[first_separator + 1..];
    let linux_path = format!(
        "/{}",
        distro_path
            .split('\\')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    );

    Some(WslProjectPath {
        distro,
        linux_path: if linux_path == "/" {
            "/".to_string()
        } else {
            linux_path
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_wsl_project_path, WslProjectPath};

    #[test]
    fn parses_wsl_localhost_path() {
        assert_eq!(
            parse_wsl_project_path(r"\\wsl.localhost\Ubuntu\home\me\project"),
            Some(WslProjectPath {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/me/project".to_string(),
            })
        );
    }

    #[test]
    fn parses_wsl_dollar_path() {
        assert_eq!(
            parse_wsl_project_path(r"\\wsl$\Debian\home\me\project"),
            Some(WslProjectPath {
                distro: "Debian".to_string(),
                linux_path: "/home/me/project".to_string(),
            })
        );
    }

    #[test]
    fn ignores_normal_unix_path() {
        assert_eq!(parse_wsl_project_path("/home/me/project"), None);
    }
}
