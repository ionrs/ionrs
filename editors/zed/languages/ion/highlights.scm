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
  "as"
] @keyword

[
  "let"
  "mut"
  "fn"
] @keyword

; ── Loop labels ──────────────────────────────────────────────
(label) @label

(labeled_loop_statement
  label: (label) @label)

(break_statement
  (label) @label)

(continue_statement
  (label) @label)

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

(field_expression
  field: (identifier) @property)

((field_expression
  field: (identifier) @keyword)
  (#eq? @keyword "await"))

(named_argument
  name: (identifier) @variable.parameter)

((identifier) @function.builtin
  (#match? @function.builtin "^(len|range|enumerate|type_of|str|int|float|bytes|bytes_from_hex|assert|assert_eq|channel|set|cell|sleep|timeout)$"))

((identifier) @namespace
  (#match? @namespace "^(math|json|io|string|log)$"))

; ── Module paths ─────────────────────────────────────────────
(module_path
  (identifier) @namespace)

(import_path_tail
  (identifier) @namespace)

(module_path_import
  (identifier) @namespace)

(import_item
  name: (identifier) @variable
  alias: (identifier)? @variable)

; ── Parameters ───────────────────────────────────────────────
(parameter
  name: (identifier) @variable.parameter)

(closure_parameters
  (identifier) @variable.parameter)

(rest_pattern
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

; F-string prefix — distinct scope so themes can highlight `f"` / `f"""`
(fstring_literal "f\"" @string.special.symbol)
(fstring_literal "f\"\"\"" @string.special.symbol)

(interpolation
  "{" @punctuation.special
  "}" @punctuation.special)

(boolean_literal) @boolean
(none_literal) @constant.builtin
(unit_literal) @constant.builtin

; ── Collection Entries ───────────────────────────────────────
(dict_entry
  key: (primary_expression (identifier) @property))

(spread_expression
  "..." @operator)

(rest_pattern
  "..." @operator)

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
"#{" @punctuation.special

; ── Closure pipes ────────────────────────────────────────────
(closure_parameters
  "|" @punctuation.bracket)

; ── Identifiers (fallback) ───────────────────────────────────
(identifier) @variable
