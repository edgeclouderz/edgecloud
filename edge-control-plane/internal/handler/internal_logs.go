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

// MaxEntries caps the number of log entries in a single batch. Matches the
// worker's `max_buffer_len * HARD_CAP_MULT = 1000` ceiling so any worker
// batch always fits. A runaway worker cannot submit unbounded rows in one
// POST even with tiny messages.
const MaxEntries = 1000

// logEntryRepo is the subset of *repository.LogEntryRepository used here.
// Defining it locally keeps tests mockable without a live DB.
type logEntryRepo interface {
	InsertBatch(ctx context.Context, entries []domain.LogEntry) error
}

// IngestLogsRequest is the JSON body the worker sends to /api/internal/logs.
//
// The JSON shape is intentionally lenient: unknown fields are accepted (a
// future worker struct drift becomes a no-op instead of a 400 that drops
// the entire batch). Syntactically broken bodies still 400.
type IngestLogsRequest struct {
	Entries []domain.LogEntry `json:"entries"`
}

// IngestLogs handles POST /api/internal/logs — tenant log ingest from workers.
//
// Auth: WorkerAuth middleware (HMAC-SHA256 JWT). The handler:
//   - caps request body at MaxLogBatchSize
//   - caps the number of entries per batch at MaxEntries
//   - overwrites each entry's TenantID, WorkerID, and Region with the JWT's
//     claims (the worker can lie in the body, but the JWT is the truth —
//     this is the security boundary that prevents a compromised worker
//     from attributing its logs to another tenant, worker, or region)
//   - trusts DeploymentID, AppName, Level, Message, Labels from the body
//
// Response: 204 No Content on success. 400 on malformed body, oversize
// batch, or too many entries. 401 is handled by WorkerAuth before the
// handler runs. 500 on DB failure.
func (h *InternalHandler) IngestLogs(w http.ResponseWriter, r *http.Request) {
	tenantID := middleware.GetWorkerTenantID(r.Context())
	workerID := middleware.GetWorkerID(r.Context())
	region := middleware.GetWorkerRegion(r.Context())

	// Cap request body before decoding. MaxBytesReader returns a
	// *http.MaxBytesError when the (N+1)-th read past the cap is attempted.
	r.Body = http.MaxBytesReader(w, r.Body, MaxLogBatchSize)
	defer r.Body.Close()

	var req IngestLogsRequest
	// Lenient decode: unknown fields are accepted so a future worker struct
	// drift doesn't drop the whole batch. Syntactically broken JSON still
	// surfaces as a decode error below.
	dec := json.NewDecoder(r.Body)
	if err := dec.Decode(&req); err != nil {
		// MaxBytesReader returns a *http.MaxBytesError when the body exceeds
		// the cap; io.EOF for empty body; everything else is shape/parse.
		// Surface all as 400 with a generic message so we don't leak parser
		// internals to the worker.
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

	// Cap the entry count so a worker that submits many tiny entries in a
	// 1 MiB body cannot blow past any per-batch row budget.
	if len(req.Entries) > MaxEntries {
		http.Error(w, `{"error": "too many entries"}`, http.StatusBadRequest)
		return
	}

	// Overwrite authoritative fields from the JWT. We trust the JWT, not
	// the body, for tenant/worker/region identity. This is the security
	// boundary: a worker that lies about TenantID/WorkerID/Region in the
	// body gets its logs filed under the JWT's values.
	for i := range req.Entries {
		req.Entries[i].TenantID = tenantID
		req.Entries[i].WorkerID = workerID
		req.Entries[i].Region = region
	}

	if err := h.logEntryRepo.InsertBatch(r.Context(), req.Entries); err != nil {
		log.Printf("ingest logs: insert failed (tenant=%s, count=%d): %v", tenantID, len(req.Entries), err)
		http.Error(w, `{"error": "internal error"}`, http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusNoContent)
}
