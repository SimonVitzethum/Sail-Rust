default Order dec
$include <prelude.sail>
val test_bits_and : forall 'n. (bits('n), bits('n)) -> bits('n)
overload operator & = {test_bits_and}
val test_not_bits : forall 'n. bits('n) -> bits('n)
val test_mask : forall 'n, 'n >= 0. (implicit('n), int, int) -> bits('n)
val masked : forall 'n, 'n >= 0. (bits('n), int, int) -> bits('n)
function masked(xs, i, j) = xs & test_not_bits(test_mask(false, j))
