default Order dec

$include <prelude.sail>

bitfield cr_type : bits(8) = {
  CR0 : 7 .. 4,
  LT  : 7,
  GT  : 6,
  CR1 : 3 .. 2,
  CR3 : 1 .. 0
}

register R : cr_type

enum E = A | B | C

struct pkt = {
  addr : bits(32),
  data : bits(64)
}

mapping size_bits = {
  BYTE   <-> 0b00,
  HALF   <-> 0b01,
  WORD   <-> 0b10,
  DOUBLE <-> 0b11
}
