#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    boltget_ipc::browser::fuzz(data);
});
