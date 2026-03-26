; Ion language — tree-sitter highlight queries for Zed

; ── Comments ─────────────────────────────────────────────────
(line_comment) @comment

; ── Keywords ─────────────────────────────────────────────────
[
  "if"
  "else"
  "match"
  "for"
  "while"
  "loop"
  "break"
  "continue"
  "return"
  "in"
  "async"
  "spawn"
  "select"
  "try"
  "catch"
  "use"
] @keyword

[
  "let"
  "mut"
  "fn"
] @keyword

; ── Constructors ─────────────────────────────────────────────
(some_expression "Some" @constructor)
(some_pattern "Some" @constructor)
(ok_expression "Ok" @constructor)
(ok_pattern "Ok" @constructor)
(err_expression "Err" @constructor)
(err_pattern "Err" @constructor)
(none_literal) @constant.builtin

; ── Functions ────────────────────────────────────────────────
(function_definition
  name: (identifier) @function)

(call_expression
  function: (primary_expression (identifier) @function))

(method_call_expression
  method: (identifier) @function.method)

; ── Module paths ─────────────────────────────────────────────
(module_path_expression
  module: (identifier) @namespace
  member: (identifier) @function)

(module_path_import
  (identifier) @namespace)

; ── Parameters ───────────────────────────────────────────────
(parameter
  name: (identifier) @variable.parameter)

(closure_parameters
  (identifier) @variable.parameter)

; ── Types ────────────────────────────────────────────────────
(type_identifier) @type.builtin

(type_annotation
  (type_identifier) @type.builtin)

; ── Literals ─────────────────────────────────────────────────
(integer_literal) @number
(float_literal) @number

(string_literal) @string
(fstring_literal) @string
(byte_literal) @string

(string_content) @string
(string_content_triple) @string

(escape_sequence) @string.escape

(interpolation
  "{" @punctuation.special
  "}" @punctuation.special)

(boolean_literal) @boolean
(none_literal) @constant.builtin
(unit_literal) @constant.builtin

; ── Operators ────────────────────────────────────────────────
[
  "+"
  "-"
  "*"
  "/"
  "%"
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "&&"
  "||"
  "!"
  "&"
  "^"
  "<<"
  ">>"
  ".."
  "..="
  "..."
  "|>"
  "?"
  "=>"
] @operator

[
  "="
  "+="
  "-="
  "*="
  "/="
] @operator

; ── Punctuation ──────────────────────────────────────────────
["(" ")" "[" "]" "{" "}"] @punctuation.bracket

[
  ","
  ";"
  ":"
  "."
] @punctuation.delimiter

"::" @punctuation.special
"#{"@punctuation.special

; ── Closure pipes ────────────────────────────────────────────
(closure_parameters
  "|" @punctuation.bracket)

; ── Identifiers (fallback) ───────────────────────────────────
(identifier) @variable
