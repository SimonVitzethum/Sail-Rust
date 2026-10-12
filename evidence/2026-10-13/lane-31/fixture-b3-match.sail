default Order dec

$include <prelude.sail>

union MyU = { MyA : int(8), MyB : bool }

function match_demo(u : MyU) -> int = {
  match u {
    MyA(x) => x,
    MyB(_) => 0
  }
}
