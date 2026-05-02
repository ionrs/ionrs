package com.ionlang.idea.highlighting

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.openapi.fileTypes.SyntaxHighlighterFactory
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile

class IonSyntaxHighlighterFactory : SyntaxHighlighterFactory() {
    override fun getSyntaxHighlighter(project: Project?, virtualFile: VirtualFile?): SyntaxHighlighter {
        LOG.info(
            "Creating Ion syntax highlighter for file=${virtualFile?.path ?: "<unknown>"} " +
                "project=${project?.basePath ?: "<none>"}",
        )
        return IonSyntaxHighlighter()
    }

    companion object {
        private val LOG = Logger.getInstance(IonSyntaxHighlighterFactory::class.java)
    }
}
