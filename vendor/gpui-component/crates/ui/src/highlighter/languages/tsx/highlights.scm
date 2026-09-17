; These patterns layer JSX syntax over the JavaScript and TypeScript base
; queries configured in languages.rs.

((jsx_opening_element
  (identifier) @type)
 (#match? @type "^[A-Z]"))

((jsx_closing_element
  (identifier) @type)
 (#match? @type "^[A-Z]"))

((jsx_self_closing_element
  (identifier) @type)
 (#match? @type "^[A-Z]"))

(jsx_opening_element
  (member_expression) @type)

(jsx_closing_element
  (member_expression) @type)

(jsx_self_closing_element
  (member_expression) @type)

((jsx_opening_element
  (identifier) @tag)
 (#match? @tag "^[a-z]"))

((jsx_closing_element
  (identifier) @tag)
 (#match? @tag "^[a-z]"))

((jsx_self_closing_element
  (identifier) @tag)
 (#match? @tag "^[a-z]"))

(jsx_attribute
  (property_identifier) @attribute)

(jsx_opening_element
  ["<" ">"] @punctuation.bracket)

(jsx_closing_element
  ["</" ">"] @punctuation.bracket)

(jsx_self_closing_element
  ["<" "/>" ] @punctuation.bracket)

(jsx_attribute
  "=" @punctuation.delimiter)

(jsx_text) @text.literal
(html_character_reference) @string.special
