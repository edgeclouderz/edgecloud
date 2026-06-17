package handler

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/middleware"
)

// MaxLogBatchSize caps the size of a single POST /api/internal/logs body.
// 1 MiB is well above any plausible batch (the worker batcher flushes every
// 1s/100 entries, and each entry is typically <1 KiB). The cap bounds per-
// request memory and per-DB-row-count; oversize requests are rejected before
// we touch the DB.
const MaxLogBatchSize = 1 << 20

// logEntryRepo is the subset of *repository.LogEntryRepository used here.
// Defining it locally keeps tests mockable without a live DB.
type logEntryRepo interface {
	InsertBatch(ctx context.Context, entries []domain.LogEntry) error
}

// IngestLogsRequest is the JSON body the worker sends to /api/internal/logs.
type IngestLogsRequest struct {
	Entries []domain.LogEntry `json:"entries"`
}

// IngestLogs handles POST /api/internal/logs — tenant log ingest from workers.
//
// Auth: WorkerAuth middleware (HMAC-SHA256 JWT). The handler:
//   - caps request body at MaxLogBatchSize
//   - overwrites each entry's TenantID with the JWT's tenant_id (the worker
//     can lie in the body, but the JWT is the truth)
//   - stamps WorkerID and Region from the JWT (so a compromised worker cannot
//     attribute its own logs to another worker)
//   - trusts DeploymentID, AppName, Level, Message, Labels from the body
//
// Response: 204 No Content on success. 400 on malformed body or oversize
// batch. 401 is handled by WorkerAuth before the handler runs. 500 on DB
// failure.
func (h *InternalHandler) IngestLogs(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetWorkerTenantID(r.Context())
	workerID := middleware.GetWorkerID(r.Context())

	// Cap request body before decoding.
	r.Body = http.MaxBytesReader(w, r.Body, MaxLogBatchSize+1)
	defer r.Body.Close()

	var req IngestLogsRequest
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields() // tightens against typos in worker clients
	if err := dec.Decode(&req); err != nil {
		// MaxBytesReader returns a *http.MaxBytesError when the body exceeds
		// the cap; json.UnmarshalTypeError for shape mismatches; io.EOF for
		// empty body. Surface all as 400 with a generic message so we don't
		// leak parser internals to the worker.
		var maxErr *http.MaxBytesError
		switch {
		case errors.As(err, &maxErr):
			http.Error(w, `{"error": "batch too large"}`, http.StatusBadRequest)
		case errors.Is(err, io.EOF):
			http.Error(w, `{"error": "empty body"}`, http.StatusBadRequest)
		default:
			http.Error(w, `{"error": "invalid request body"}`, http.StatusBadRequest)
		}
		return
	}

	// Reject empty batches early — saves a roundtrip and matches the
	// repository's no-op behavior.
	if len(req.Entries) == 0 {
		w.WriteHeader(http.StatusNoContent)
		return
	}

	// Overwrite authoritative fields from the JWT. We trust the JWT, not
	// the body, for tenant/worker identity. This is the security boundary:
	// a worker that lies about TenantID in the body gets its logs filed
	// under the JWT's tenant_id.
	for i := range req.Entries {
		req.Entries[i].TenantID = tenantID
		req.Entries[i].WorkerID = workerID
	}

	if err := h.logEntryRepo.InsertBatch(r.Context(), req.Entries); err != nil {
		log.Printf("ingest logs: insert failed (tenant=%s, count=%d): %v", tenantID, len(req.Entries), err)
		http.Error(w, `{"error": "internal error"}`, http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusNoContent)
}
