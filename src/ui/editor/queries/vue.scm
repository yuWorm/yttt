(comment) @comment

(tag_name) @tag
(erroneous_end_tag_name) @tag.error
(doctype) @constant
(attribute_name) @attribute
(attribute (attribute_value) @string)
(attribute (quoted_attribute_value (attribute_value) @string))

(directive_name) @keyword
(directive_value) @attribute
(dynamic_directive_inner_value) @variable
(directive_modifier) @function.method

[
  ":"
  "."
  "@"
  "#"
] @punctuation.delimiter

[
  "<"
  ">"
  "</"
  "/>"
] @punctuation.bracket
