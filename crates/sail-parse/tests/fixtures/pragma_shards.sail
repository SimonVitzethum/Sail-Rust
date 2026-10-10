$include_start "outer.sail"

function f(x) = {
  // comment with { unbalanced brace
  /* block comment with } and $pragma inside */
  let s = "string with { } and $x inside";
  x + 1
}
and f(0) = {
  0
}

$[no_enum_number_conversions]
enum E = A | B | C

$opt { level = 2, mode = "fast" }

scattered function g

scattered end g

$include_end "outer.sail"
