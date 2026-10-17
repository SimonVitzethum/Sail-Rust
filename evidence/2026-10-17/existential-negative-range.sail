default Order dec
$include <prelude.sail>
val outside : unit -> { 'n, 0 <= 'n & 'n < 4. int('n) }
function outside() = 4
