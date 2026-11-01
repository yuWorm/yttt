(comment) @comment
(bool_lit) @boolean
(null_lit) @constant.builtin
(numeric_lit) @number
(template_literal) @string

[
  (quoted_template_start)
  (quoted_template_end)
  (heredoc_start)
  (heredoc_identifier)
] @punctuation.delimiter

[
  (template_interpolation_start)
  (template_interpolation_end)
  (template_directive_start)
  (template_directive_end)
  (strip_marker)
] @punctuation.special

(attribute
  (identifier) @property)

(block
  (identifier) @keyword)

(block
  (string_lit) @string)

(function_call
  (identifier) @function)

(get_attr
  (identifier) @property)

(variable_expr
  (identifier) @variable)

(for_intro
  "for" @keyword.repeat
  "in" @keyword.repeat)

(for_cond
  "if" @keyword.conditional)

[
  "!"
  "*"
  "/"
  "%"
  "+"
  "-"
  ">"
  ">="
  "<"
  "<="
  "=="
  "!="
  "&&"
  "||"
] @operator

[
  "{"
  "}"
  "["
  "]"
  "("
  ")"
] @punctuation.bracket

[
  "."
  ".*"
  ","
  "[*]"
  "="
  ":"
] @punctuation.delimiter

[
  (ellipsis)
  "?"
  "=>"
] @punctuation.special
