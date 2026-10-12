default Order dec

$include <prelude.sail>

function slice_demo(b : bits(16)) -> bits(8) = b[15 .. 8]
