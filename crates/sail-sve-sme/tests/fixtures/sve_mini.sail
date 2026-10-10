val execute_MINI : forall 'VL 'dn 'esize, (int('VL), int('dn), int('esize)) -> unit

function execute_MINI (VL, dn, esize) = {
    CheckSVEEnabled();
    let mask : bits(8) = P_read(g, PL);
    result : bits('VL) = undefined;
    foreach (e from 0 to (elements - 1) by 1 in inc) {
        let element1 : bits('esize) = Elem_read(operand1, e, esize);
        result = Elem_set(result, e, esize, element1)
    };
    Z_set(dn, VL) = result
}

function decode_MINI (size) = {
    if not_bool(HaveSVE()) then {
        throw(Error_Undefined())
    };
    assert(constraint('esize in {16, 32}));
    assert(false);
    match VL {
      128 => { execute_MINI(128, dn, esize) },
      _ => { throw(Error_Undefined()) }
    }
}
