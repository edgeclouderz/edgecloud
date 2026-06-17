package service

import (
	"bytes"
	"io"
	"strings"
	"testing"
)

// minimalWasmHeader is the 8-byte preamble every WebAssembly module and
// WASI Preview 2 component starts with.
var minimalWasmHeader = []byte{0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00}

func TestValidateWasmModule(t *testing.T) {
	tests := []struct {
		name      string
		data      []byte
		wantError bool
		errSubstr string
	}{
		{
			name:      "valid wasm header only",
			data:      minimalWasmHeader,
			wantError: false,
		},
		{
			name:      "valid wasm header with trailing bytes",
			data:      append(minimalWasmHeader, 0xde, 0xad, 0xbe, 0xef),
			wantError: false,
		},
		{
			name:      "empty input",
			data:      []byte{},
			wantError: true,
			errSubstr: "0 bytes",
		},
		{
			name:      "too short (7 bytes)",
			data:      minimalWasmHeader[:7],
			wantError: true,
			errSubstr: "7 bytes",
		},
		{
			name:      "wrong magic (text)",
			data:      append([]byte("not a wasm module"), bytes.Repeat([]byte{0}, 100)...),
			wantError: true,
			errSubstr: "magic bytes",
		},
		{
			name:      "elf binary",
			data:      append([]byte{0x7f, 'E', 'L', 'F'}, bytes.Repeat([]byte{0}, 100)...),
			wantError: true,
			errSubstr: "magic bytes",
		},
		{
			name:      "unsupported wasm version",
			data:      []byte{0x00, 0x61, 0x73, 0x6d, 0x02, 0x00, 0x00, 0x00},
			wantError: true,
			errSubstr: "unsupported WASM version",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := ValidateWasmModule(tt.data)
			if tt.wantError {
				if err == nil {
					t.Fatalf("expected error, got nil")
				}
				if tt.errSubstr != "" && !strings.Contains(err.Error(), tt.errSubstr) {
					t.Errorf("error %q does not contain %q", err.Error(), tt.errSubstr)
				}
			} else if err != nil {
				t.Errorf("unexpected error: %v", err)
			}
		})
	}
}

// TestDeployRejectsNonWasm ensures the Deploy pipeline rejects non-Wasm
// artifacts before any DB write or storage call. This is the end-to-end
// guarantee behind issue #35.
func TestDeployRejectsNonWasm(t *testing.T) {
	// Build a DeploymentService without DB or storage dependencies; Deploy must
	// reject before touching them.
	svc := &DeploymentService{}

	cases := []struct {
		name string
		data []byte
	}{
		{"text file", []byte("hello world, this is not a wasm module")},
		{"empty body", []byte{}},
		{"7-byte stub", []byte{0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := svc.Deploy(t.Context(), "t_1", "my-app", io.NopCloser(bytes.NewReader(tc.data)))
			if err == nil {
				t.Fatalf("expected error for non-wasm input, got nil")
			}
			if !strings.Contains(err.Error(), "WebAssembly") {
				t.Errorf("error %q should mention WebAssembly", err.Error())
			}
		})
	}
}
