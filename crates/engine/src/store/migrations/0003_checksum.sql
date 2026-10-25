-- Schema version 3: checksum verification.
ALTER TABLE download ADD COLUMN checksum_algo TEXT;     -- 'md5' | 'sha1' | 'sha256' | 'sha512'
ALTER TABLE download ADD COLUMN checksum_expected TEXT; -- lowercase hex
ALTER TABLE download ADD COLUMN checksum_source TEXT;   -- 'user', or the response header it came from
ALTER TABLE download ADD COLUMN checksum_actual TEXT;   -- lowercase hex, once verified (or found corrupt)
