use crate::dll::win32::Win32Context;
use chardetng::EncodingDetector;
use encoding_rs::EUC_KR;
use unicorn_engine::Unicorn;

use super::memory::read_string_bytes_impl;

pub(crate) fn write_euc_kr_impl(uc: &mut Unicorn<Win32Context>, addr: u64, text: &str) {
    let (encoded, _, _) = EUC_KR.encode(text);
    let bytes = encoded.as_ref();
    let _ = uc.mem_write(addr, bytes);
    let _ = uc.mem_write(addr + bytes.len() as u64, &[0u8]);
}

pub(crate) fn read_euc_kr_impl(uc: &Unicorn<Win32Context>, addr: u64) -> String {
    let bytes = read_string_bytes_impl(uc, addr, 2048);
    if bytes.is_empty() {
        return String::new();
    }

    // EUC-KR 디코딩 (필요한 경우에만 수행)
    let filtered: Vec<u8> = bytes.iter().filter(|&&b| b > 127).copied().collect();
    if filtered.is_empty() {
        return String::from_utf8_lossy(&bytes).to_string();
    }

    let mut detector = EncodingDetector::new();
    detector.feed(&filtered, true);
    let encoding = detector.guess(None, true);

    if encoding.name().contains("UTF") {
        String::from_utf8_lossy(&bytes).to_string()
    } else {
        let (res, _, _) = EUC_KR.decode(&bytes);
        res.to_string()
    }
}
