package com.ionlang.idea.lsp

import com.intellij.openapi.project.Project
import com.ionlang.idea.settings.IonSettings
import com.redhat.devtools.lsp4ij.server.CannotStartProcessException
import com.redhat.devtools.lsp4ij.server.ProcessStreamConnectionProvider
import java.io.File

class IonLanguageServer(project: Project) : ProcessStreamConnectionProvider() {
    init {
        val command = IonSettings.instance.lspPath.ifBlank { DEFAULT_BINARY }
        super.setCommands(listOf(command))
        project.basePath?.let { super.setWorkingDirectory(it) }
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
                On Windows + WSL, RustRover spawns processes from Windows, so either install
                ion-lsp.exe on the Windows side or configure the path to invoke WSL
                (e.g. `wsl` with arg `ion-lsp`). To silence this warning, uncheck
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
    }
}
