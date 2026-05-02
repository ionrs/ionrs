# Ion Language — JetBrains plugin

Syntax highlighting and LSP support for the [Ion scripting
language](../) in JetBrains IDEs (IntelliJ IDEA, RustRover, PyCharm, WebStorm,
etc., Community or Ultimate, 2024.2+).

This is the JetBrains sibling of the [VSCode](../vscode/) and [Zed](../zed/)
extensions and ships the same `tmLanguage` grammar.

## Features

- File-type registration for `.ion`
- Syntax highlighting via TextMate grammar (`source.ion`)
- Bracket pairing and `//` line comments
- Optional language-server integration through
  [LSP4IJ](https://plugins.jetbrains.com/plugin/23257-lsp4ij), driving an
  external `ion-lsp` binary

## Build

```bash
cd editors/jetbrains
./gradlew buildPlugin
```

Output: `build/distributions/ion-jetbrains-0.7.5.zip`. Install in any JetBrains
IDE via *Settings | Plugins | ⚙ | Install Plugin from Disk…*.

## Run a sandbox IDE

```bash
./gradlew runIde
```

Open any `.ion` file (e.g.
`sensors/ion-sensor/scripts/*.ion`) inside the sandbox to verify highlighting.

## Configure the language server

The plugin depends on [LSP4IJ](https://plugins.jetbrains.com/plugin/23257-lsp4ij)
(installed automatically as a required dependency). Once `ion-lsp` is on your
PATH, LSP4IJ launches it for every `.ion` file.

Version 0.7.3 raises the LSP4IJ dependency to a 2025-era build. That matters for
Ctrl+Q / quick documentation in TextMate-backed `.ion` files; older LSP4IJ
builds could start the server yet still fail to route hover responses into
JetBrains documentation popups.

Version 0.7.4 also normalizes custom WSL commands like
`wsl.exe -d Ubuntu --cd /home/me/project ion-lsp` into a shell launch that adds
`$HOME/.cargo/bin` to `PATH`, matching where `cargo install` places `ion-lsp`.

Version 0.7.5 registers `.ion` as a native JetBrains file type and maps that
file type directly to LSP4IJ. This avoids cases where TextMate highlighting is
present but the LSP server is not attached to the editor.

Install `ion-lsp`:

```bash
cargo install --path ion-lsp
```

Adjust the binary path or disable LSP entirely under
**Settings | Languages & Frameworks | Ion**.

### Windows + WSL

If your IDE runs on Windows and the project lives in WSL
(`\\wsl.localhost\...`), the IDE process spawns from Windows and won't see
binaries installed inside WSL. The plugin now detects WSL UNC project paths and,
when the LSP path is left as `ion-lsp`, launches:

```text
wsl.exe -d <distro> --cd <project-path> -- sh -c 'PATH="$HOME/.cargo/bin:$PATH"; export PATH; exec ion-lsp'
```

You can also:

- install `ion-lsp.exe` on the Windows side, **or**
- set the LSP path to a full command line such as
  `wsl.exe -d Ubuntu --cd /home/me/project -- sh -c 'PATH="$HOME/.cargo/bin:$PATH"; export PATH; exec ion-lsp'`, **or**
- turn off "Enable Ion language server" in the Ion settings panel and use the
  plugin for syntax highlighting only.

## Updating the syntax grammar

The TextMate grammar is the source of truth in
`../vscode/syntaxes/ion.tmLanguage.json`. After updating it there, copy it over:

```bash
cp ../vscode/syntaxes/ion.tmLanguage.json \
   src/main/resources/ion-bundle/Syntaxes/ion.tmLanguage.json
```

Re-add the `"fileTypes": ["ion"]` field if it gets dropped — it tells the
JetBrains TextMate plugin which file extensions the grammar applies to.

## Layout

```
jetbrains/
├── build.gradle.kts            IntelliJ Platform Gradle Plugin v2 config
├── settings.gradle.kts
├── gradle.properties
└── src/main/
    ├── kotlin/com/ionlang/idea/
    │   ├── IonTextMateBundleProvider.kt   register the bundled grammar
    │   ├── lsp/
    │   │   ├── IonLanguageServer.kt       spawn ion-lsp
    │   │   └── IonLanguageServerFactory.kt
    │   └── settings/
    │       ├── IonSettings.kt             persisted lspPath / lspEnabled
    │       └── IonConfigurable.kt         settings UI
    └── resources/
        ├── META-INF/plugin.xml
        ├── META-INF/pluginIcon.svg
        └── ion-bundle/
            ├── info.plist
            ├── Syntaxes/ion.tmLanguage.json
            └── Preferences/ion.tmPreferences
```
