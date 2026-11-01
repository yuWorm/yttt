; Keep every language fixed with #set!: the local highlighter presently does
; not resolve @injection.language captures.

((script_element
  (start_tag) @_start
  (raw_text) @injection.content)
(#not-match? @_start "[[:space:]]lang[[:space:]]*=")
 (#set! injection.language "javascript"))

((script_element
  (start_tag
    (attribute
      (attribute_name) @_language_name
      (quoted_attribute_value
        (attribute_value) @_language)))
  (raw_text) @injection.content)
 (#eq? @_language_name "lang")
(#any-of? @_language "js" "javascript")
 (#set! injection.language "javascript"))

((script_element
  (start_tag
    (attribute
      (attribute_name) @_language_name
      (quoted_attribute_value
        (attribute_value) @_language)))
  (raw_text) @injection.content)
 (#eq? @_language_name "lang")
(#any-of? @_language "ts" "typescript")
 (#set! injection.language "typescript"))

((script_element
  (start_tag
    (attribute
      (attribute_name) @_language_name
      (quoted_attribute_value
        (attribute_value) @_language)))
  (raw_text) @injection.content)
 (#eq? @_language_name "lang")
 (#any-of? @_language "tsx" "jsx")
 (#set! injection.language "tsx"))

((style_element
  (start_tag) @_start
  (raw_text) @injection.content)
(#not-match? @_start "[[:space:]]lang[[:space:]]*=")
 (#set! injection.language "css"))

((style_element
  (start_tag
    (attribute
      (attribute_name) @_language_name
      (quoted_attribute_value
        (attribute_value) @_language)))
  (raw_text) @injection.content)
 (#eq? @_language_name "lang")
 (#eq? @_language "css")
 (#set! injection.language "css"))

((style_element
  (start_tag
    (attribute
      (attribute_name) @_language_name
      (quoted_attribute_value
        (attribute_value) @_language)))
  (raw_text) @injection.content)
 (#eq? @_language_name "lang")
 (#eq? @_language "scss")
 (#set! injection.language "scss"))

((interpolation
  (raw_text) @injection.content)
 (#set! injection.language "typescript"))

((directive_attribute
  (quoted_attribute_value
    (attribute_value) @injection.content))
 (#set! injection.language "typescript"))
