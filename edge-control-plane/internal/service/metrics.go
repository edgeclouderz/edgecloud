package service

import (
	"fmt"
	"strings"
	"sync"

	"github.com/edgeclouderz/edge-cloud/edge-control-plane/internal/domain"
)

// MetricsAggregator collects per-app metric samples pushed via heartbeats
// and renders them as Prometheus text-format output.
//
// Data is held in memory only — no DB persistence. Each heartbeat replaces
// the previous batch for a given (tenantID, appName) pair, matching the
// worker's subtract_delta model where counters reflect the delta since the
// last heartbeat rather than a cumulative total.
type MetricsAggregator struct {
	mu sync.RWMutex
	// tenantID → appName → []sample
	data map[string]map[string]appMetrics
}

type appMetrics struct {
	requestCount  uint64
	outboundBytes uint64
	samples       []domain.MetricSample
}

// NewMetricsAggregator returns a ready-to-use aggregator.
func NewMetricsAggregator() *MetricsAggregator {
	return &MetricsAggregator{
		data: make(map[string]map[string]appMetrics),
	}
}

// Ingest records the metric samples for one (tenantID, appName) pair reported
// in a heartbeat. It also ingests the built-in request_count and
// outbound_bytes so all per-app metrics are served from one place.
func (a *MetricsAggregator) Ingest(tenantID, appName string, requestCount, outboundBytes uint64, samples []domain.MetricSample) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if _, ok := a.data[tenantID]; !ok {
		a.data[tenantID] = make(map[string]appMetrics)
	}
	a.data[tenantID][appName] = appMetrics{
		requestCount:  requestCount,
		outboundBytes: outboundBytes,
		samples:       samples,
	}
}

// RenderTenant returns a Prometheus text-format string containing only the
// metrics for the given tenant. Returns an empty string when no data has
// been ingested for that tenant yet.
func (a *MetricsAggregator) RenderTenant(tenantID string) string {
	a.mu.RLock()
	defer a.mu.RUnlock()
	apps, ok := a.data[tenantID]
	if !ok {
		return ""
	}
	var b strings.Builder
	renderApps(&b, tenantID, apps)
	return b.String()
}

// RenderAll returns a Prometheus text-format string containing metrics for
// all tenants. Used by the unauthenticated GET /metrics operator endpoint.
func (a *MetricsAggregator) RenderAll() string {
	a.mu.RLock()
	defer a.mu.RUnlock()
	var b strings.Builder
	for tenantID, apps := range a.data {
		renderApps(&b, tenantID, apps)
	}
	return b.String()
}

// renderApps writes Prometheus lines for every app belonging to one tenant.
func renderApps(b *strings.Builder, tenantID string, apps map[string]appMetrics) {
	for appName, m := range apps {
		baseLabels := fmt.Sprintf(`tenant_id=%q,app=%q`, tenantID, appName)

		// Built-in platform counters (always present, even when observer_metrics
		// is empty — they come from RequestMeter, not edge:observe).
		fmt.Fprintf(b, "# TYPE edge_request_count counter\n")
		fmt.Fprintf(b, "edge_request_count{%s} %d\n", baseLabels, m.requestCount)
		fmt.Fprintf(b, "# TYPE edge_outbound_bytes counter\n")
		fmt.Fprintf(b, "edge_outbound_bytes{%s} %d\n", baseLabels, m.outboundBytes)

		// Guest-emitted metrics from edge:observe.
		for _, s := range m.samples {
			labelStr := buildLabelStr(baseLabels, s.Labels)
			switch s.Kind {
			case domain.MetricKindCounter:
				fmt.Fprintf(b, "# TYPE edge_counter counter\n")
				fmt.Fprintf(b, "edge_counter{%s,metric=%q} %g\n", labelStr, s.Name, s.Value)
			case domain.MetricKindGauge:
				fmt.Fprintf(b, "# TYPE edge_gauge gauge\n")
				fmt.Fprintf(b, "edge_gauge{%s,metric=%q} %g\n", labelStr, s.Name, s.Value)
			case domain.MetricKindHistogramSample:
				fmt.Fprintf(b, "# TYPE edge_histogram_sample untyped\n")
				fmt.Fprintf(b, "edge_histogram_sample{%s,metric=%q} %g\n", labelStr, s.Name, s.Value)
			}
		}
	}
}

// buildLabelStr prepends the base labels with any extra labels from the sample.
func buildLabelStr(base string, extra [][2]string) string {
	if len(extra) == 0 {
		return base
	}
	var parts []string
	for _, kv := range extra {
		parts = append(parts, fmt.Sprintf("%s=%q", kv[0], kv[1]))
	}
	return base + "," + strings.Join(parts, ",")
}
