/* Fixture: drei val/function-Paare, Miniatur von sysregs_autogen.sail. */
val ACCDATA_EL1_SysRegRead_aaa : forall 't.
  int('t) -> int

function ACCDATA_EL1_SysRegRead_aaa (el, op0) = {
    if op0 == 0b01 then {
        return(1)
    };
    return(0)
}

val ACTLR_EL1_SysRegRead_bbb : forall 't.
  int('t) -> int

function ACTLR_EL1_SysRegRead_bbb (el, op0) = {
    return(op0)
}

val AFSR0_EL1_SysRegRead_ccc : forall 't.
  int('t) -> int

function AFSR0_EL1_SysRegRead_ccc (el, op0) = {
    return(op0)
}
