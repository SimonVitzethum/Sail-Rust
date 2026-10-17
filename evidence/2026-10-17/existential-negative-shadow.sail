default Order dec
$include <prelude.sail>
val produce : forall 'n, 'n > 0. int('n) -> { 'n, 'n == 0. int('n) }
val wrong : int(42) -> int(42)
function wrong(n) = produce(n)
