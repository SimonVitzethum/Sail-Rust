val execute_TILE : forall 'VL, (int('VL)) -> unit

function execute_TILE (VL) = {
    CheckStreamingSVEAndZAEnabled();
    foreach (row from 0 to (dim - 1) by 1 in inc) {
        foreach (col from 0 to (dim - 1) by 1 in inc) {
            let element1 : bits(32) = Elem_read(operand1, row, esize)
        }
    };
    let tile : bits(64) = ZAtile_read(VL, da, esize, n)
}
