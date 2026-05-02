package com.ionlang.idea.lsp

import com.intellij.openapi.project.Project
import com.ionlang.idea.settings.IonSettings
import com.redhat.devtools.lsp4ij.server.CannotStartProcessException
import com.redhat.devtools.lsp4ij.server.ProcessStreamConnectionProvider
import java.io.File

class IonLanguageServer(project: Project) : ProcessStreamConnectionProvider() {
    init {
        val command = commandForProject(IonSettings.instance.lspPath, project.basePath)
        super.setCommands(command)
        if (!command.isWslInvocation()) {
            project.basePath?.let { super.setWorkingDirectory(it) }
        }
    }

    override fun start() {
        if (!IonSettings.instance.lspEnabled) {
            throw CannotStartProcessException(
                "Ion language server is disabled in Settings | Languages & Frameworks | Ion.",
            )
        }
        val command = commands?.firstOrNull()
            ?: throw CannotStartProcessException("Ion language server command is not configured.")
        if (resolveOnPath(command) == null) {
            throw CannotStartProcessException(
                """
                Ion language server binary '$command' was not found.
                Install it with `cargo install --path ion-lsp` (or `cargo install ion-lsp`),
                then set the absolute path under Settings | Languages & Frameworks | Ion.
                On Windows + WSL, JetBrains IDEs spawn processes from Windows. Open projects
                under \\wsl.localhost\Distro\... are launched through wsl.exe automatically
                when the path is left as '$DEFAULT_BINARY'. You can also configure a full
                command line such as `wsl.exe -d Ubuntu --cd /home/me/project -- sh -c 'PATH="${'$'}HOME/.cargo/bin:${'$'}PATH"; export PATH; exec ion-lsp'`.
                To silence this warning, uncheck
                "Enable Ion language server" in the same settings panel.
                """.trimIndent(),
            )
        }
        super.start()
    }

    private fun resolveOnPath(command: String): File? {
        val asFile = File(command)
        if (asFile.isAbsolute) {
            return if (asFile.isFile && asFile.canExecute()) asFile else null
        }
        val pathEnv = System.getenv("PATH") ?: return null
        val pathExt = System.getenv("PATHEXT")?.split(File.pathSeparator)?.filter { it.isNotEmpty() }.orEmpty()
        val suffixes = if (pathExt.isEmpty()) listOf("") else listOf("") + pathExt
        for (dir in pathEnv.split(File.pathSeparator)) {
            for (suffix in suffixes) {
                val candidate = File(dir, command + suffix)
                if (candidate.isFile && candidate.canExecute()) return candidate
            }
        }
        return null
    }

    companion object {
        private const val DEFAULT_BINARY = "ion-lsp"
        private const val WSL_DEFAULT_COMMAND = "PATH=\"\$HOME/.cargo/bin:\$PATH\"; export PATH; exec ion-lsp"

        private fun commandForProject(
            configuredPath: String,
            projectBasePath: String?,
        ): List<String> {
            val commandLine = configuredPath.trim().ifEmpty { DEFAULT_BINARY }
            if (isWindows() && commandLine == DEFAULT_BINARY) {
                parseWslProjectPath(projectBasePath)?.let { wslPath ->
                    return listOf(
                        windowsWslExecutable(),
                        "-d",
                        wslPath.distro,
                        "--cd",
                        wslPath.linuxPath,
                        "--",
                        "sh",
                        "-c",
                        WSL_DEFAULT_COMMAND,
                    )
                }
            }
            return commandPartsFromSetting(commandLine).ifEmpty { listOf(DEFAULT_BINARY) }
        }

        private fun windowsWslExecutable(): String {
            val windowsRoot = System.getenv("SystemRoot")
                ?: System.getenv("WINDIR")
                ?: return "wsl.exe"
            val candidate = File(windowsRoot, "System32\\wsl.exe")
            return if (candidate.isFile) candidate.absolutePath else "wsl.exe"
        }

        private fun List<String>.isWslInvocation(): Boolean =
            firstOrNull()?.let { executableName(it) }?.let { executable ->
                executable.equals("wsl", ignoreCase = true) ||
                    executable.equals("wsl.exe", ignoreCase = true)
            } == true

        private fun executableName(command: String): String =
            command.substringAfterLast('\\').substringAfterLast('/')

        private fun commandPartsFromSetting(value: String): List<String> {
            val parts = splitCommandLine(value)
            if (parts.size <= 1) return parts

            val first = parts.first()
            if (
                value.startsWith('"') ||
                value.startsWith('\'') ||
                first.equals("wsl", ignoreCase = true) ||
                first.equals("wsl.exe", ignoreCase = true)
            ) {
                return parts
            }

            return listOf(value)
        }

        private fun splitCommandLine(value: String): List<String> {
            val parts = mutableListOf<String>()
            val current = StringBuilder()
            var quote: Char? = null

            for (ch in value) {
                if ((ch == '\'' || ch == '"') && quote == null) {
                    quote = ch
                    continue
                }
                if (ch == quote) {
                    quote = null
                    continue
                }
                if (ch.isWhitespace() && quote == null) {
                    if (current.isNotEmpty()) {
                        parts += current.toString()
                        current.clear()
                    }
                    continue
                }
                current.append(ch)
            }

            if (current.isNotEmpty()) {
                parts += current.toString()
            }
            return parts
        }

        private fun isWindows(): Boolean =
            System.getProperty("os.name").contains("Windows", ignoreCase = true)

        private data class WslProjectPath(val distro: String, val linuxPath: String)

        private fun parseWslProjectPath(path: String?): WslProjectPath? {
            if (path.isNullOrBlank()) return null
            val normalized = path.replace('/', '\\')
            val wslLocalhostPrefix = "\\\\wsl.localhost\\"
            val wslDollarPrefix = "\\\\wsl\$\\"
            val prefixLength = when {
                normalized.startsWith(wslLocalhostPrefix, ignoreCase = true) ->
                    wslLocalhostPrefix.length
                normalized.startsWith(wslDollarPrefix, ignoreCase = true) ->
                    wslDollarPrefix.length
                else -> return null
            }
            val rest = normalized.substring(prefixLength)
            val firstSeparator = rest.indexOf('\\')
            if (firstSeparator <= 0) return null
            val distro = rest.substring(0, firstSeparator)
            val distroPath = rest.substring(firstSeparator + 1)
            val linuxPath = "/" + distroPath
                .split('\\')
                .filter { it.isNotEmpty() }
                .joinToString("/")
            return WslProjectPath(distro, linuxPath.ifBlank { "/" })
        }
    }
}
