package httperror

import (
	"encoding/json"
	"net/http"
)

// ErrorCode is a machine-readable SCREAMING_SNAKE_CASE identifier.
type ErrorCode string

const (
	CodeBadRequest    ErrorCode = "BAD_REQUEST"
	CodeUnauthorized  ErrorCode = "UNAUTHORIZED"
	CodeForbidden     ErrorCode = "FORBIDDEN"
	CodeNotFound      ErrorCode = "NOT_FOUND"
	CodeConflict      ErrorCode = "CONFLICT"
	CodeQuotaExceeded ErrorCode = "QUOTA_EXCEEDED"
	CodeInternalError ErrorCode = "INTERNAL_ERROR"
)

// ErrorResponse is the canonical JSON error envelope.
// All 4xx responses include the real message. All 5xx responses
// return message "internal error" to avoid leaking internals.
type ErrorResponse struct {
	Error ErrorDetail `json:"error"`
}

type ErrorDetail struct {
	Code    ErrorCode `json:"code"`
	Message string    `json:"message"`
}

func write(w http.ResponseWriter, code ErrorCode, message string, httpStatus int) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(httpStatus)
	json.NewEncoder(w).Encode(ErrorResponse{Error: ErrorDetail{Code: code, Message: message}})
}

// BadRequest reports a malformed request (HTTP 400).
func BadRequest(w http.ResponseWriter, message string) {
	write(w, CodeBadRequest, message, http.StatusBadRequest)
}

// Unauthorized reports a missing or invalid credential (HTTP 401).
func Unauthorized(w http.ResponseWriter, message string) {
	write(w, CodeUnauthorized, message, http.StatusUnauthorized)
}

// Forbidden reports insufficient permissions (HTTP 403).
func Forbidden(w http.ResponseWriter, message string) {
	write(w, CodeForbidden, message, http.StatusForbidden)
}

// NotFound reports a missing resource (HTTP 404).
func NotFound(w http.ResponseWriter, message string) {
	write(w, CodeNotFound, message, http.StatusNotFound)
}

// Conflict reports a state conflict such as duplicate creation (HTTP 409).
func Conflict(w http.ResponseWriter, message string) {
	write(w, CodeConflict, message, http.StatusConflict)
}

// QuotaExceeded reports a quota limit hit (HTTP 429).
func QuotaExceeded(w http.ResponseWriter, message string) {
	write(w, CodeQuotaExceeded, message, http.StatusTooManyRequests)
}

// InternalError reports an unspecified server fault (HTTP 500).
// Use this when logging has already captured the real error; the client
// always sees "internal error" regardless of what happened.
func InternalError(w http.ResponseWriter) {
	write(w, CodeInternalError, "internal error", http.StatusInternalServerError)
}
