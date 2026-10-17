default Order dec
$include <prelude.sail>
val within : unit -> { 'n, 0 <= 'n & 'n < 4. int('n) }
function within() = 1
val small : unit -> { 'n, 0 <= 'n & 'n < 4. int('n) }
val wider : unit -> { 'm, 0 <= 'm & 'm < 8. int('m) }
function wider() = small()
val produce : forall 'n, 'n > 0. int('n) -> { 'r, 0 <= 'r & 'r < 'n. int('r) }
val upper : forall 'm, 'm > 0. int('m) -> { 's, 0 <= 's & 's < 'm + 1. int('s) }
function upper(m) = produce(m)
val shadow : forall 'n, 'n > 0. int('n) -> { 'n, 'n == 0. int('n) }
val zero : int(42) -> int(0)
function zero(n) = shadow(n)
