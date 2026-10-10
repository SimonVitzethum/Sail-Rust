enum Feature = {
  FEAT_AA32EL0,
  FEAT_AA64EL0,
  FEAT_SVE
}

bitfield DormantCtl_Type : bits(32) =
  {
    dbgrstreq : 12,
    standbywfi : 0
  }

register PSTATE_C : bool

register SEE : int

type AccType = enum { AccType_NORMAL, AccType_STREAM }

let min_int8 = -128

val HaveCSSC : unit -> bool

function HaveCSSC () = {
    boolean IMPLEMENTATION_DEFINED "Have CSSC"
}

val FPExpCoefficient_read : int -> bits(32)

function FPExpCoefficient_read (n) = {
    match n {
    0 => 0x00000000,
    1 => 0x0e7ec568,
    2 => 0x1cc5da28,
    _ => 0x00000000
    }
}

overload X_read = { X_read32, X_read64 }
