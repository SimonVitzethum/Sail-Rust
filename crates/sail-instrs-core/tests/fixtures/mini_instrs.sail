val execute_aarch32_instrs_ADC_i_Op_A_txt : (int, bits(32), int, bool) -> unit

function execute_aarch32_instrs_ADC_i_Op_A_txt (d, imm32, n, setflags) = {
    nzcv : bits(4) = undefined;
    result : bits(32) = AddWithCarry(R_read(n), imm32, PSTATE.C);
    if d == 15 then {
        if setflags then {
            ALUExceptionReturn(result)
        } else {
            ALUWritePC(result)
        }
    } else {
        R_set(d) = result
    };
    ()
}

val execute_aarch32_instrs_VMOV_i_Op_A_txt : (int, bits(32)) -> unit

function execute_aarch32_instrs_VMOV_i_Op_A_txt (d, imm32) = {
    match d {
    15 => ALUWritePC(imm32),
    _ => R_set(d) = imm32
    }
}

val decode_aarch32_instrs_ADC_i_A1enc_A_txt : (bits(4), bits(1), bits(4), bits(4), bits(12)) -> unit

function decode_aarch32_instrs_ADC_i_A1enc_A_txt (cond, S, Rn, Rd, imm12) = {
    if ConditionPassed() then {
        let 'd = UInt(Rd);
        let 'n = UInt(Rn);
        let imm32 : bits(32) = A32ExpandImm(imm12);
        execute_aarch32_instrs_ADC_i_Op_A_txt(d, imm32, n, S == 0b1)
    };
    ()
}

val decode_aarch32_instrs_ADC_i_T1enc_A_txt : (bits(1), bits(1), bits(4), bits(3), bits(8)) -> unit

function decode_aarch32_instrs_ADC_i_T1enc_A_txt (S, Rn, imm3, Rd, imm8) = {
    if ConditionPassed() then {
        execute_aarch32_instrs_ADC_i_Op_A_txt(UInt(Rd), T32ExpandImm(imm3 @ imm8), UInt(Rn), S == 0b1)
    };
    ()
}

val decode_aarch32_instrs_VMOV_i_A1enc_A_txt : (bits(4), bits(12)) -> unit

function decode_aarch32_instrs_VMOV_i_A1enc_A_txt (Rd, imm12) = {
    if ConditionPassed() then {
        execute_aarch32_instrs_VMOV_i_Op_A_txt(UInt(Rd), A32ExpandImm(imm12))
    };
    ()
}

function clause __DecodeA32_DataProMisc ((pc, ([_, _, _, _, bitzero, bitzero, bitone, bitzero, bitone, bitzero, bitone, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _] as __opcode)) if Slice(__opcode, 28, 4) != 0b1111 & SEE < 2813) = {
    SEE = 2813;
    let cond = Slice(__opcode, 28, 4);
    decode_aarch32_instrs_ADC_i_A1enc_A_txt(cond, Slice(__opcode, 20, 1), Slice(__opcode, 16, 4), Slice(__opcode, 12, 4), Slice(__opcode, 0, 12))
}

function clause __DecodeT32 ((pc, ([bitone, bitone, bitone, bitzero, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _] as __opcode)) if SEE < 2814) = {
    SEE = 2814;
    decode_aarch32_instrs_ADC_i_T1enc_A_txt(Slice(__opcode, 26, 1), Slice(__opcode, 20, 1), Slice(__opcode, 16, 4), Slice(__opcode, 12, 3), Slice(__opcode, 0, 8))
}

function clause __DecodeT32 ((pc, ([bitone, bitzero, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _] as __opcode)) if SEE < 2815) = {
    SEE = 2815;
    decode_aarch32_instrs_VMOV_i_A1enc_A_txt(Slice(__opcode, 12, 4), Slice(__opcode, 0, 12))
}
