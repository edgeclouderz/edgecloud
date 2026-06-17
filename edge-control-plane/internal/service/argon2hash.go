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

	version, err := parsePHCVersion(parts[2])
	if err != nil {
		return false, err
	}
	if version != argon2.Version {
		return false, fmt.Errorf("argon2: unsupported version %d", version)
	}

	memory, time, threads, err := parsePHCParams(parts[3])
	if err != nil {
		return false, err
	}
	if memory == 0 || time == 0 || threads == 0 {
		return false, fmt.Errorf("argon2: parameters must be non-zero (got m=%d, t=%d, p=%d)", memory, time, threads)
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

// parsePHCVersion parses a "v=N" segment. Hand-rolled rather than using
// fmt.Sscanf so a malformed input like "v=abc" or "v=19junk" produces an
// unambiguous error and we don't accept partial matches.
func parsePHCVersion(s string) (int, error) {
	if !strings.HasPrefix(s, "v=") {
		return 0, fmt.Errorf("argon2: bad version %q (want v=N)", s)
	}
	n, err := parseUintDecimal(s[2:])
	if err != nil {
		return 0, fmt.Errorf("argon2: bad version %q: %w", s, err)
	}
	return n, nil
}

// parsePHCParams parses an "m=<mem>,t=<time>,p=<threads>" segment.
// fmt.Sscanf is notoriously lenient (matches partial input, swallows
// trailing junk); a hand-rolled parser lets us fail loudly on malformed
// strings.
func parsePHCParams(s string) (memory, time uint32, threads uint8, err error) {
	for _, kv := range strings.Split(s, ",") {
		k, v, ok := strings.Cut(kv, "=")
		if !ok {
			return 0, 0, 0, fmt.Errorf("argon2: bad parameter segment %q (want key=value)", kv)
		}
		switch k {
		case "m":
			n, err := parseUintDecimal(v)
			if err != nil {
				return 0, 0, 0, fmt.Errorf("argon2: bad m=%q: %w", v, err)
			}
			memory = uint32(n)
		case "t":
			n, err := parseUintDecimal(v)
			if err != nil {
				return 0, 0, 0, fmt.Errorf("argon2: bad t=%q: %w", v, err)
			}
			time = uint32(n)
		case "p":
			n, err := parseUintDecimal(v)
			if err != nil {
				return 0, 0, 0, fmt.Errorf("argon2: bad p=%q: %w", v, err)
			}
			threads = uint8(n)
		default:
			return 0, 0, 0, fmt.Errorf("argon2: unknown parameter %q", k)
		}
	}
	return memory, time, threads, nil
}

// parseUintDecimal parses a non-negative decimal integer without leading
// sign or whitespace — strict enough that "19junk" fails rather than
// silently truncating.
func parseUintDecimal(s string) (int, error) {
	if s == "" {
		return 0, fmt.Errorf("empty number")
	}
	n := 0
	for i, c := range s {
		if c < '0' || c > '9' {
			return 0, fmt.Errorf("invalid digit %q at position %d", c, i)
		}
		n = n*10 + int(c-'0')
	}
	return n, nil
}
