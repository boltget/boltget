#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    boltget_media::pgp::fuzz(data);
});
