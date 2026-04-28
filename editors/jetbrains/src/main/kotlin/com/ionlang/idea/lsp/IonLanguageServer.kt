package com.ionlang.idea.lsp

import com.intellij.openapi.project.Project
import com.ionlang.idea.settings.IonSettings
import com.redhat.devtools.lsp4ij.server.CannotStartProcessException
import com.redhat.devtools.lsp4ij.server.ProcessStreamConnectionProvider

class IonLanguageServer(project: Project) : ProcessStreamConnectionProvider() {
    init {
        val command = IonSettings.instance.lspPath.ifBlank { "ion-lsp" }
        super.setCommands(listOf(command))
        project.basePath?.let { super.setWorkingDirectory(it) }
    }

    override fun start() {
        if (!IonSettings.instance.lspEnabled) {
            throw CannotStartProcessException(
                "Ion language server is disabled in Settings | Languages & Frameworks | Ion.",
            )
        }
        super.start()
    }
}
