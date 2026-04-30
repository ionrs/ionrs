/// <reference types="tree-sitter-cli/dsl" />
// Tree-sitter grammar for the Ion programming language
// https://github.com/chutuananh2k/ion-lang

const PREC = {
  ASSIGN: 1,
  PIPE_FWD: 2,
  OR: 3,
  AND: 4,
  BIT_OR: 5,
  BIT_XOR: 6,
  BIT_AND: 7,
  EQUAL: 8,
  COMPARE: 9,
  SHIFT: 10,
  RANGE: 11,
  ADD: 12,
  MUL: 13,
  UNARY: 14,
  TRY: 15,
  CALL: 16,
  FIELD: 17,
  INDEX: 18,
};

module.exports = grammar({
  name: "ion",

  extras: ($) => [/\s/, $.line_comment],

  word: ($) => $.identifier,

  conflicts: ($) => [
    [$.primary_expression, $.named_argument],
    [$.method_call_expression, $.field_expression],
  ],

  rules: {
    source_file: ($) => repeat($._statement),

    // ── Statements ──────────────────────────────────────────────

    _statement: ($) =>
      choice(
        $.let_statement,
        $.function_definition,
        $.use_statement,
        $.expression_statement,
        $.break_statement,
        $.continue_statement,
        $.return_statement,
      ),

    let_statement: ($) =>
      seq(
        "let",
        optional("mut"),
        $.pattern,
        optional(seq(":", $.type_annotation)),
        "=",
        $._expression,
        ";",
      ),

    function_definition: ($) =>
      seq(
        "fn",
        field("name", $.identifier),
        "(",
        optional($.parameter_list),
        ")",
        $.block,
      ),

    parameter_list: ($) =>
      seq($.parameter, repeat(seq(",", $.parameter)), optional(",")),

    parameter: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq("=", field("default", $._expression))),
      ),

    use_statement: ($) =>
      seq(
        "use",
        $.module_path_import,
        ";",
      ),

    module_path_import: ($) =>
      choice(
        // use mod::name
        seq($.identifier, "::", $.identifier),
        // use mod::{a, b}
        seq($.identifier, "::", "{", $.import_list, "}"),
        // use mod::*
        seq($.identifier, "::", "*"),
      ),

    import_list: ($) =>
      seq($.identifier, repeat(seq(",", $.identifier)), optional(",")),

    expression_statement: ($) =>
      seq($._expression, optional(";")),

    break_statement: ($) =>
      seq("break", optional($._expression), ";"),

    continue_statement: ($) =>
      seq("continue", ";"),

    return_statement: ($) =>
      seq("return", optional($._expression), ";"),

    // ── Types ───────────────────────────────────────────────────

    type_annotation: ($) =>
      choice(
        $.type_identifier,
        seq($.type_identifier, "<", $.type_annotation, repeat(seq(",", $.type_annotation)), ">"),
      ),

    type_identifier: ($) =>
      choice(
        "int",
        "float",
        "bool",
        "string",
        "bytes",
        "list",
        "dict",
        "tuple",
        "set",
        "fn",
        "any",
        "Option",
        "Result",
      ),

    // ── Expressions ─────────────────────────────────────────────

    _expression: ($) =>
      choice(
        $.binary_expression,
        $.unary_expression,
        $.try_expression,
        $.assignment_expression,
        $.call_expression,
        $.method_call_expression,
        $.field_expression,
        $.index_expression,
        $.module_path_expression,
        $.if_expression,
        $.match_expression,
        $.closure_expression,
        $.block,
        $.primary_expression,
        $.list_expression,
        $.tuple_expression,
        $.dict_expression,
        $.list_comprehension,
        $.dict_comprehension,
        $.loop_expression,
        $.while_expression,
        $.for_expression,
        $.async_expression,
        $.spawn_expression,
        $.select_expression,
        $.try_catch_expression,
      ),

    primary_expression: ($) =>
      choice(
        $.identifier,
        $.integer_literal,
        $.float_literal,
        $.string_literal,
        $.fstring_literal,
        $.byte_literal,
        $.boolean_literal,
        $.none_literal,
        $.unit_literal,
        $.some_expression,
        $.ok_expression,
        $.err_expression,
        $.spread_expression,
      ),

    // ── Literals ────────────────────────────────────────────────

    integer_literal: (_) => /[0-9][0-9_]*/,

    float_literal: (_) => /[0-9][0-9_]*\.[0-9][0-9_]*/,

    string_literal: ($) =>
      choice(
        seq('"""', repeat(choice($.escape_sequence, $.string_content_triple)), '"""'),
        seq('"', repeat(choice($.escape_sequence, $.string_content)), '"'),
      ),

    fstring_literal: ($) =>
      choice(
        seq('f"""', repeat(choice($.interpolation, $.escape_sequence, $.string_content_triple)), '"""'),
        seq('f"', repeat(choice($.interpolation, $.escape_sequence, $.string_content)), '"'),
      ),

    byte_literal: ($) =>
      seq('b"', repeat(choice($.escape_sequence, $.string_content)), '"'),

    string_content: (_) => /[^"\\{]+/,
    string_content_triple: (_) => /[^"\\{]+/,

    escape_sequence: (_) =>
      /\\[nrt0\\"']|\\x[0-9a-fA-F]{2}|\\u\{[0-9a-fA-F]+\}/,

    interpolation: ($) =>
      seq("{", $._expression, "}"),

    boolean_literal: (_) => choice("true", "false"),

    none_literal: (_) => "None",

    unit_literal: (_) => seq("(", ")"),

    some_expression: ($) =>
      seq("Some", "(", $._expression, ")"),

    ok_expression: ($) =>
      seq("Ok", "(", $._expression, ")"),

    err_expression: ($) =>
      seq("Err", "(", $._expression, ")"),

    spread_expression: ($) =>
      seq("...", $._expression),

    // ── Collections ─────────────────────────────────────────────

    list_expression: ($) =>
      seq("[", optional(seq($._expression, repeat(seq(",", $._expression)), optional(","))), "]"),

    tuple_expression: ($) =>
      seq("(", $._expression, ",", optional(seq($._expression, repeat(seq(",", $._expression)))), optional(","), ")"),

    dict_expression: ($) =>
      seq("#{", optional($.dict_entry_list), "}"),

    dict_entry_list: ($) =>
      seq($.dict_entry, repeat(seq(",", $.dict_entry)), optional(",")),

    dict_entry: ($) =>
      seq(field("key", $._expression), ":", field("value", $._expression)),

    list_comprehension: ($) =>
      seq(
        "[",
        $._expression,
        "for",
        $.pattern,
        "in",
        $._expression,
        optional(seq("if", $._expression)),
        "]",
      ),

    dict_comprehension: ($) =>
      seq(
        "#{",
        $._expression, ":", $._expression,
        "for",
        $.pattern,
        "in",
        $._expression,
        optional(seq("if", $._expression)),
        "}",
      ),

    // ── Operators ───────────────────────────────────────────────

    binary_expression: ($) =>
      choice(
        prec.left(PREC.PIPE_FWD, seq($._expression, "|>", $._expression)),
        prec.left(PREC.OR, seq($._expression, "||", $._expression)),
        prec.left(PREC.AND, seq($._expression, "&&", $._expression)),
        prec.left(PREC.BIT_OR, seq($._expression, "|", $._expression)),
        prec.left(PREC.BIT_XOR, seq($._expression, "^", $._expression)),
        prec.left(PREC.BIT_AND, seq($._expression, "&", $._expression)),
        prec.left(PREC.EQUAL, seq($._expression, choice("==", "!="), $._expression)),
        prec.left(PREC.COMPARE, seq($._expression, choice("<", ">", "<=", ">="), $._expression)),
        prec.left(PREC.SHIFT, seq($._expression, choice("<<", ">>"), $._expression)),
        prec.left(PREC.RANGE, seq($._expression, choice("..", "..="), $._expression)),
        prec.left(PREC.ADD, seq($._expression, choice("+", "-"), $._expression)),
        prec.left(PREC.MUL, seq($._expression, choice("*", "/", "%"), $._expression)),
      ),

    unary_expression: ($) =>
      prec(PREC.UNARY, seq(choice("-", "!"), $._expression)),

    try_expression: ($) =>
      prec(PREC.TRY, seq($._expression, "?")),

    assignment_expression: ($) =>
      prec.right(
        PREC.ASSIGN,
        seq($._expression, choice("=", "+=", "-=", "*=", "/="), $._expression),
      ),

    // ── Access ──────────────────────────────────────────────────

    call_expression: ($) =>
      prec(PREC.CALL, seq(
        field("function", $._expression),
        "(",
        optional($.argument_list),
        ")",
      )),

    method_call_expression: ($) =>
      prec(PREC.FIELD, seq(
        $._expression,
        ".",
        field("method", $.identifier),
        "(",
        optional($.argument_list),
        ")",
      )),

    argument_list: ($) =>
      seq(
        choice($.named_argument, $._expression),
        repeat(seq(",", choice($.named_argument, $._expression))),
        optional(","),
      ),

    named_argument: ($) =>
      seq(field("name", $.identifier), "=", field("value", $._expression)),

    field_expression: ($) =>
      prec(PREC.FIELD, seq($._expression, ".", field("field", $.identifier))),

    index_expression: ($) =>
      prec(PREC.INDEX, seq(
        $._expression,
        "[",
        choice(
          $._expression,
          $.slice_range,
        ),
        "]",
      )),

    slice_range: ($) =>
      seq(
        optional($._expression),
        choice("..", "..="),
        optional($._expression),
      ),

    module_path_expression: ($) =>
      seq(
        field("module", $.identifier),
        "::",
        field("member", $.identifier),
      ),

    // ── Control Flow ────────────────────────────────────────────

    if_expression: ($) =>
      prec.right(seq(
        "if",
        field("condition", $._expression),
        field("consequence", $.block),
        optional(seq(
          "else",
          field("alternative", choice($.block, $.if_expression)),
        )),
      )),

    match_expression: ($) =>
      seq(
        "match",
        field("subject", $._expression),
        "{",
        optional(seq($.match_arm, repeat(seq(",", $.match_arm)), optional(","))),
        "}",
      ),

    match_arm: ($) =>
      seq(
        $.pattern,
        optional(seq("if", field("guard", $._expression))),
        "=>",
        $._expression,
      ),

    // ── Patterns ────────────────────────────────────────────────

    pattern: ($) =>
      choice(
        $.identifier,
        $.integer_literal,
        $.float_literal,
        $.string_literal,
        $.boolean_literal,
        $.none_literal,
        "_",
        $.tuple_pattern,
        $.list_pattern,
        $.some_pattern,
        $.ok_pattern,
        $.err_pattern,
        $.rest_pattern,
      ),

    tuple_pattern: ($) =>
      seq("(", $.pattern, repeat1(seq(",", $.pattern)), optional(","), ")"),

    list_pattern: ($) =>
      seq("[", optional(seq($.pattern, repeat(seq(",", $.pattern)), optional(","))), "]"),

    some_pattern: ($) =>
      seq("Some", "(", $.pattern, ")"),

    ok_pattern: ($) =>
      seq("Ok", "(", $.pattern, ")"),

    err_pattern: ($) =>
      seq("Err", "(", $.pattern, ")"),

    rest_pattern: ($) =>
      seq("...", $.identifier),

    // ── Closures ────────────────────────────────────────────────

    closure_expression: ($) =>
      prec(-1, seq(
        $.closure_parameters,
        $._expression,
      )),

    closure_parameters: ($) =>
      seq(
        "|",
        optional(seq($.identifier, repeat(seq(",", $.identifier)), optional(","))),
        "|",
      ),

    // ── Blocks ──────────────────────────────────────────────────

    block: ($) =>
      seq("{", repeat($._statement), "}"),

    // ── Loop expressions (when used as values) ───────────────────

    loop_expression: ($) =>
      seq("loop", $.block),

    while_expression: ($) =>
      seq("while", $._expression, $.block),

    for_expression: ($) =>
      seq("for", $.pattern, "in", $._expression, $.block),

    // ── Concurrency ─────────────────────────────────────────────

    async_expression: ($) =>
      seq("async", $.block),

    spawn_expression: ($) =>
      seq("spawn", $._expression),

    select_expression: ($) =>
      seq(
        "select",
        "{",
        repeat($.select_arm),
        "}",
      ),

    select_arm: ($) =>
      seq($.pattern, "=", $._expression, "=>", $._expression, optional(",")),

    try_catch_expression: ($) =>
      seq(
        "try",
        $.block,
        "catch",
        optional(choice(
          seq("(", $.identifier, ")"),
          $.identifier,
        )),
        $.block,
      ),

    // ── Terminals ───────────────────────────────────────────────

    identifier: (_) => /[a-zA-Z_][a-zA-Z0-9_]*/,

    line_comment: (_) => /\/\/[^\n]*/,
  },
});
