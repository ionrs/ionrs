package com.ionlang.idea

import com.intellij.openapi.fileTypes.LanguageFileType
import javax.swing.Icon

class IonFileType private constructor() : LanguageFileType(IonLanguage) {
    override fun getName(): String = "Ion"

    override fun getDescription(): String = "Ion source file"

    override fun getDefaultExtension(): String = "ion"

    override fun getIcon(): Icon? = null

    companion object {
        @JvmField
        val INSTANCE = IonFileType()
    }
}
