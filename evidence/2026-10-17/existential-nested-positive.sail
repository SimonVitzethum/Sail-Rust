default Order dec
$include <prelude.sail>
val produce : unit -> { 'n, 'n == 0. { 'n, 'n == 1. int('n) } }
val one : unit -> int(1)
function one() = produce()
