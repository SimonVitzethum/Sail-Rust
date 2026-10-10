default Order dec

$include <prelude.sail>

val bitvector_eq : (bits(16), bits(16)) -> bool
function bitvector_eq(x, y) = {
  x == y
}

$include <arith.sail>

register R : bits(64)

type addr = bits(64)

overload foo = { bar, baz }

infixr 6 ^
