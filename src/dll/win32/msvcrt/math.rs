use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::{RegisterX86, Unicorn};

// =========================================================
// Math
// =========================================================
// API: double floor(double x)
// 역할: 지정된 값보다 작거나 같은 최대 정수를 계산
pub(super) fn floor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let x_low = uc.read_arg(0);
    let x_high = uc.read_arg(1);
    let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
    let res = x.floor();
    crate::emu_log!("[MSVCRT] floor({}) -> double {}", x, res);
    // FIXME: ST(0)에 결과 기록 필요. 현재는 EAX=0 리턴
    Some(ApiHookResult::callee(2, Some(0)))
}

// API: double ceil(double x)
// 역할: 지정된 값보다 크거나 같은 최소 정수를 계산
pub(super) fn ceil(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let x_low = uc.read_arg(0);
    let x_high = uc.read_arg(1);
    let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
    let res = x.ceil();
    crate::emu_log!("[MSVCRT] ceil({}) -> double {}", x, res);
    // FIXME: ST(0)에 결과 기록 필요. 현재는 EAX=0 리턴
    Some(ApiHookResult::callee(2, Some(0)))
}

pub(super) fn frexp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let x_low = uc.read_arg(0);
    let x_high = uc.read_arg(1);
    let exp_ptr = uc.read_arg(2);
    let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
    // Simple dummy: x = m * 2^e
    uc.write_u32(exp_ptr as u64, 0);
    crate::emu_log!("[MSVCRT] frexp({}, {:#x}) -> double {}", x, exp_ptr, x);
    Some(ApiHookResult::callee(3, Some(0)))
}

pub(super) fn ldexp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let x_low = uc.read_arg(0);
    let x_high = uc.read_arg(1);
    let exp = uc.read_arg(2) as i32;
    let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
    let res = x * 2.0f64.powi(exp);
    crate::emu_log!("[MSVCRT] ldexp({}, {}) -> double {}", x, exp, res);
    Some(ApiHookResult::callee(3, Some(0)))
}

pub(super) fn _ftol(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    // _ftol: ST(0)를 정수로 변환하여 EAX에 저장
    // x87 레지스터 ST0 읽기 (Unicorn은 보통 f64 비트로 변환하여 반환하거나 하위 64비트 반환)
    let raw_val = uc.reg_read(RegisterX86::ST0).unwrap_or(0);
    let val_f = f64::from_bits(raw_val);
    let res = val_f as i32;

    crate::emu_log!(
        "[MSVCRT] _ftol: ST(0) bits={:#x} ({}) -> EAX={}",
        raw_val,
        val_f,
        res
    );
    Some(ApiHookResult::callee(0, Some(res)))
}

pub(super) fn __c_ipow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    // _CIpow: ST(1) ^ ST(0) 계산 후 ST(1)에 저장하고 ST(0) 팝
    let st0_bits = uc.reg_read(RegisterX86::ST0).unwrap_or(0);
    let st1_bits = uc.reg_read(RegisterX86::ST1).unwrap_or(0);

    let st0 = f64::from_bits(st0_bits);
    let st1 = f64::from_bits(st1_bits);

    let res = st1.powf(st0);
    let res_bits = res.to_bits();

    // ST(1)에 결과 기록 (Unicorn x86에서 ST(1) 쓰기가 정확히 동작하는지 확인 필요)
    let _ = uc.reg_write(RegisterX86::ST1, res_bits);
    // ST(0)을 팝해야 하지만, 여기서는 단순히 결과 기록만 시도
    // (실제로는 FPU TOP 포인터를 조작해야 함)

    crate::emu_log!("[MSVCRT] _CIpow: {} ^ {} -> {}", st1, st0, res);
    Some(ApiHookResult::callee(0, Some(0)))
}
