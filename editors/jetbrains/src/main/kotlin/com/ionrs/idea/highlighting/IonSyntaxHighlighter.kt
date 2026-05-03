package com.ionrs.idea.highlighting

import com.intellij.lexer.Lexer
import com.intellij.openapi.editor.DefaultLanguageHighlighterColors
import com.intellij.openapi.editor.HighlighterColors
import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighterBase
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

class IonSyntaxHighlighter : SyntaxHighlighterBase() {
    override fun getHighlightingLexer(): Lexer = IonLexer()

    override fun getTokenHighlights(tokenType: IElementType): Array<TextAttributesKey> =
        pack(ATTRIBUTES[tokenType])

    companion object {
        private val COMMENT = TextAttributesKey.createTextAttributesKey("ION_COMMENT", DefaultLanguageHighlighterColors.LINE_COMMENT)
        private val STRING = TextAttributesKey.createTextAttributesKey("ION_STRING", DefaultLanguageHighlighterColors.STRING)
        private val NUMBER = TextAttributesKey.createTextAttributesKey("ION_NUMBER", DefaultLanguageHighlighterColors.NUMBER)
        private val KEYWORD = TextAttributesKey.createTextAttributesKey("ION_KEYWORD", DefaultLanguageHighlighterColors.KEYWORD)
        private val TYPE = TextAttributesKey.createTextAttributesKey("ION_TYPE", DefaultLanguageHighlighterColors.CLASS_NAME)
        private val BUILTIN = TextAttributesKey.createTextAttributesKey("ION_BUILTIN", DefaultLanguageHighlighterColors.PREDEFINED_SYMBOL)
        private val OPERATOR = TextAttributesKey.createTextAttributesKey("ION_OPERATOR", DefaultLanguageHighlighterColors.OPERATION_SIGN)
        private val PUNCTUATION = TextAttributesKey.createTextAttributesKey("ION_PUNCTUATION", DefaultLanguageHighlighterColors.PARENTHESES)
        private val BAD_CHARACTER = TextAttributesKey.createTextAttributesKey("ION_BAD_CHARACTER", HighlighterColors.BAD_CHARACTER)

        private val ATTRIBUTES = mapOf(
            IonTokenTypes.COMMENT to COMMENT,
            IonTokenTypes.STRING to STRING,
            IonTokenTypes.NUMBER to NUMBER,
            IonTokenTypes.KEYWORD to KEYWORD,
            IonTokenTypes.TYPE to TYPE,
            IonTokenTypes.BUILTIN to BUILTIN,
            IonTokenTypes.OPERATOR to OPERATOR,
            IonTokenTypes.PUNCTUATION to PUNCTUATION,
            IonTokenTypes.BAD_CHARACTER to BAD_CHARACTER,
            TokenType.BAD_CHARACTER to BAD_CHARACTER,
        )
    }
}
