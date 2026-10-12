default Order dec

$include <prelude.sail>

function vec_assign_demo(v : vector(4, bits(8)), x : bits(8)) -> vector(4, bits(8)) = {
  var res = v;
  res[0] = x;
  res
}
