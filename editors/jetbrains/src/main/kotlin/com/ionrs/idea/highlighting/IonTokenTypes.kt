package com.ionrs.idea.highlighting

import com.intellij.psi.tree.IElementType
import com.ionrs.idea.IonLanguage

object IonTokenTypes {
    @JvmField val COMMENT = IElementType("ION_COMMENT", IonLanguage)
    @JvmField val STRING = IElementType("ION_STRING", IonLanguage)
    @JvmField val NUMBER = IElementType("ION_NUMBER", IonLanguage)
    @JvmField val KEYWORD = IElementType("ION_KEYWORD", IonLanguage)
    @JvmField val TYPE = IElementType("ION_TYPE", IonLanguage)
    @JvmField val BUILTIN = IElementType("ION_BUILTIN", IonLanguage)
    @JvmField val IDENTIFIER = IElementType("ION_IDENTIFIER", IonLanguage)
    @JvmField val OPERATOR = IElementType("ION_OPERATOR", IonLanguage)
    @JvmField val PUNCTUATION = IElementType("ION_PUNCTUATION", IonLanguage)
    @JvmField val BAD_CHARACTER = IElementType("ION_BAD_CHARACTER", IonLanguage)
}
