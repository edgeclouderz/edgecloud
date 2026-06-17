package service

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"fmt"
	"strings"

	"golang.org/x/crypto/argon2"
)

// Argon2id parameters for API key hashing. These are the OWASP-recommended
// "interactive" defaults: tuned to take ~50-100 ms on a modern server CPU.
//
// Bump memory_cost if your hardware supports it; lowering it weakens the hash.
const (
	argonTime    uint32 = 1
	argonMemory  uint32 = 64 * 1024 // 64 MiB
	argonThreads uint8  = 4
	argonSaltLen        = 16
	argonKeyLen         = 32
)

// HashAPIKey returns a PHC-formatted argon2id encoded hash of the raw API key.
//
// Format (compatible with libsodium / passlib):
//
//	$argon2id$v=19$m=65536,t=1,p=4$<base64-salt>$<base64-key>
func HashAPIKey(rawKey string) (string, error) {
	if rawKey == "" {
		return "", errors.New("argon2: empty key")
	}
	salt := make([]byte, argonSaltLen)
	if _, err := rand.Read(salt); err != nil {
		return "", fmt.Errorf("argon2: reading random salt: %w", err)
	}
	key := argon2.IDKey([]byte(rawKey), salt, argonTime, argonMemory, argonThreads, argonKeyLen)
	return fmt.Sprintf(
		"$argon2id$v=%d$m=%d,t=%d,p=%d$%s$%s",
		argon2.Version,
		argonMemory, argonTime, argonThreads,
		base64.RawStdEncoding.EncodeToString(salt),
		base64.RawStdEncoding.EncodeToString(key),
	), nil
}

// VerifyAPIKey reports whether rawKey matches the previously-encoded hash.
// Returns an error if the encoded string is malformed.
func VerifyAPIKey(rawKey, encoded string) (bool, error) {
	parts := strings.Split(encoded, "$")
	// expected: ["", "argon2id", "v=19", "m=...,t=...,p=...", "salt", "key"]
	if len(parts) != 6 || parts[0] != "" || parts[1] != "argon2id" {
		return false, fmt.Errorf("argon2: malformed encoded hash")
	}

	var version int
	if _, err := fmt.Sscanf(parts[2], "v=%d", &version); err != nil {
		return false, fmt.Errorf("argon2: bad version: %w", err)
	}
	if version != argon2.Version {
		return false, fmt.Errorf("argon2: unsupported version %d", version)
	}

	var memory uint32
	var time uint32
	var threads uint8
	if _, err := fmt.Sscanf(parts[3], "m=%d,t=%d,p=%d", &memory, &time, &threads); err != nil {
		return false, fmt.Errorf("argon2: bad parameters: %w", err)
	}

	salt, err := base64.RawStdEncoding.DecodeString(parts[4])
	if err != nil {
		return false, fmt.Errorf("argon2: bad salt: %w", err)
	}
	want, err := base64.RawStdEncoding.DecodeString(parts[5])
	if err != nil {
		return false, fmt.Errorf("argon2: bad key: %w", err)
	}

	got := argon2.IDKey([]byte(rawKey), salt, time, memory, threads, uint32(len(want)))
	return subtle.ConstantTimeCompare(got, want) == 1, nil
}
