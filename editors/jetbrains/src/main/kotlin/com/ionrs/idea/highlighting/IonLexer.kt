package com.ionrs.idea.highlighting

import com.intellij.lexer.LexerBase
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

class IonLexer : LexerBase() {
    private var buffer: CharSequence = ""
    private var endOffset: Int = 0
    private var tokenStart: Int = 0
    private var tokenEnd: Int = 0
    private var tokenType: IElementType? = null

    override fun start(buffer: CharSequence, startOffset: Int, endOffset: Int, initialState: Int) {
        this.buffer = buffer
        this.endOffset = endOffset
        tokenStart = startOffset
        locateToken()
    }

    override fun getState(): Int = 0

    override fun getTokenType(): IElementType? = tokenType

    override fun getTokenStart(): Int = tokenStart

    override fun getTokenEnd(): Int = tokenEnd

    override fun advance() {
        tokenStart = tokenEnd
        locateToken()
    }

    override fun getBufferSequence(): CharSequence = buffer

    override fun getBufferEnd(): Int = endOffset

    private fun locateToken() {
        if (tokenStart >= endOffset) {
            tokenEnd = endOffset
            tokenType = null
            return
        }

        val ch = buffer[tokenStart]
        when {
            ch.isWhitespace() -> readWhile(TokenType.WHITE_SPACE) { it.isWhitespace() }
            ch == '/' && peek(1) == '/' -> readLineComment()
            ch == '"' || ((ch == 'f' || ch == 'b') && peek(1) == '"') -> readString(if (ch == '"') tokenStart else tokenStart + 1)
            ch.isDigit() -> readNumber()
            isIdentifierStart(ch) -> readIdentifier()
            isOperatorStart(ch) -> readOperator()
            isPunctuation(ch) -> {
                tokenEnd = tokenStart + 1
                tokenType = IonTokenTypes.PUNCTUATION
            }
            else -> {
                tokenEnd = tokenStart + 1
                tokenType = IonTokenTypes.BAD_CHARACTER
            }
        }
    }

    private fun readWhile(type: IElementType, predicate: (Char) -> Boolean) {
        var index = tokenStart
        while (index < endOffset && predicate(buffer[index])) index++
        tokenEnd = index
        tokenType = type
    }

    private fun readLineComment() {
        var index = tokenStart + 2
        while (index < endOffset && buffer[index] != '\n' && buffer[index] != '\r') index++
        tokenEnd = index
        tokenType = IonTokenTypes.COMMENT
    }

    private fun readString(quoteOffset: Int) {
        val triple = quoteOffset + 2 < endOffset && buffer[quoteOffset + 1] == '"' && buffer[quoteOffset + 2] == '"'
        var index = quoteOffset + if (triple) 3 else 1
        while (index < endOffset) {
            if (!triple && buffer[index] == '\\') {
                index = (index + 2).coerceAtMost(endOffset)
                continue
            }
            if (triple && index + 2 < endOffset && buffer[index] == '"' && buffer[index + 1] == '"' && buffer[index + 2] == '"') {
                index += 3
                break
            }
            if (!triple && buffer[index] == '"') {
                index++
                break
            }
            index++
        }
        tokenEnd = index
        tokenType = IonTokenTypes.STRING
    }

    private fun readNumber() {
        var index = tokenStart
        while (index < endOffset && (buffer[index].isDigit() || buffer[index] == '_')) index++
        if (index < endOffset && buffer[index] == '.' && peek(index - tokenStart + 1)?.isDigit() == true) {
            index++
            while (index < endOffset && (buffer[index].isDigit() || buffer[index] == '_')) index++
        }
        tokenEnd = index
        tokenType = IonTokenTypes.NUMBER
    }

    private fun readIdentifier() {
        var index = tokenStart + 1
        while (index < endOffset && isIdentifierPart(buffer[index])) index++
        val word = buffer.subSequence(tokenStart, index).toString()
        tokenEnd = index
        tokenType = when (word) {
            in KEYWORDS -> IonTokenTypes.KEYWORD
            in TYPES -> IonTokenTypes.TYPE
            in BUILTINS -> IonTokenTypes.BUILTIN
            else -> IonTokenTypes.IDENTIFIER
        }
    }

    private fun readOperator() {
        var index = tokenStart + 1
        while (index < endOffset && isOperatorStart(buffer[index])) index++
        tokenEnd = index
        tokenType = IonTokenTypes.OPERATOR
    }

    private fun peek(delta: Int): Char? {
        val index = tokenStart + delta
        return if (index < endOffset) buffer[index] else null
    }

    private fun isIdentifierStart(ch: Char): Boolean = ch == '_' || ch.isLetter()

    private fun isIdentifierPart(ch: Char): Boolean = ch == '_' || ch.isLetterOrDigit()

    private fun isOperatorStart(ch: Char): Boolean = ch in "+-*/%=!<>&|^?:."

    private fun isPunctuation(ch: Char): Boolean = ch in "()[]{};,::"

    companion object {
        private val KEYWORDS = setOf(
            "let", "mut", "fn", "if", "else", "while", "for", "loop", "break", "continue",
            "return", "match", "in", "async", "spawn", "await", "select", "try", "catch",
            "use", "as", "true", "false", "None", "Some", "Ok", "Err",
        )
        private val TYPES = setOf(
            "int", "float", "bool", "string", "bytes", "list", "dict", "tuple", "set",
            "cell", "any", "Option", "Result",
        )
        private val BUILTINS = setOf(
            "len", "range", "enumerate", "type_of", "str", "int", "float", "assert",
            "assert_eq", "channel", "set", "cell", "sleep", "timeout",
            "math", "json", "io", "string", "log", "semver", "os", "path", "fs",
        )
    }
}
