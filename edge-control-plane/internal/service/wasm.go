package service

import "fmt"

// ValidateWasmModule rejects anything that does not begin with the WebAssembly
// binary magic (\0asm) and version 1 header. Plain Wasm core modules and WASI
// Preview 2 components both start with the same 8-byte preamble:
//
//	\x00asm  (magic)
//	\x01\x00\x00\x00  (version 1, little-endian)
//
// Anything else (ELF, text, image, zip, garbage) is rejected before it
// reaches storage.
func ValidateWasmModule(data []byte) error {
	const (
		wasmMagic   = "\x00asm"
		wasmVersion = 0x01
	)
	if len(data) < 8 {
		return fmt.Errorf("not a valid WebAssembly module (artifact is %d bytes, need at least 8)", len(data))
	}
	if string(data[:4]) != wasmMagic {
		return fmt.Errorf("not a valid WebAssembly module (missing \\0asm magic bytes)")
	}
	if data[4] != wasmVersion {
		return fmt.Errorf("unsupported WASM version 0x%02x (expected 0x%02x)", data[4], wasmVersion)
	}
	return nil
}
