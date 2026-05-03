package com.ionrs.idea

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerEvent
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.fileTypes.FileTypeManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.StartupActivity

class IonStartupActivity : StartupActivity.DumbAware {
    override fun runActivity(project: Project) {
        logIonProbe(project)
        project.messageBus.connect().subscribe(
            FileEditorManagerListener.FILE_EDITOR_MANAGER,
            object : FileEditorManagerListener {
                override fun fileOpened(source: FileEditorManager, file: com.intellij.openapi.vfs.VirtualFile) {
                    if (file.extension.equals("ion", ignoreCase = true)) {
                        LOG.info("Opened Ion file ${file.path}: fileType=${file.fileType.name} class=${file.fileType.javaClass.name}")
                    }
                }

                override fun selectionChanged(event: FileEditorManagerEvent) {
                    val file = event.newFile ?: return
                    if (file.extension.equals("ion", ignoreCase = true)) {
                        LOG.info("Selected Ion file ${file.path}: fileType=${file.fileType.name} class=${file.fileType.javaClass.name}")
                    }
                }
            },
        )
    }

    private fun logIonProbe(project: Project) {
        val manager = FileTypeManager.getInstance()
        val probed = manager.getFileTypeByFileName("probe.ion")
        LOG.info(
            "Ion plugin active for project=${project.basePath ?: "<none>"}; " +
                "probe.ion fileType=${probed.name} class=${probed.javaClass.name}",
        )
        FileEditorManager.getInstance(project).openFiles
            .filter { it.extension.equals("ion", ignoreCase = true) }
            .forEach { file ->
                LOG.info("Already open Ion file ${file.path}: fileType=${file.fileType.name} class=${file.fileType.javaClass.name}")
            }
    }

    companion object {
        private val LOG = Logger.getInstance(IonStartupActivity::class.java)
    }
}
