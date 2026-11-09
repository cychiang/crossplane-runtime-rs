// Command fake-external stands in for the external system that both parity
// providers manage. See parity/README.md for the contract.
package main

import (
	"bytes"
	"encoding/json"
	"io"
	"log"
	"net/http"
	"os"
	"strings"
	"sync"
)

type Thing struct {
	ConfigurableField string `json:"configurableField"`
}

// Call is one logged request. It has no timestamp so two runs compare equal.
// UserAgent and ForwardedFor let a differ tell the provider's calls from the
// scenarios' calls, which arrive through the API server's service proxy.
type Call struct {
	Seq          int             `json:"seq"`
	Method       string          `json:"method"`
	Path         string          `json:"path"`
	Body         json.RawMessage `json:"body,omitempty"`
	Status       int             `json:"status"`
	Fault        bool            `json:"fault,omitempty"`
	UserAgent    string          `json:"userAgent"`
	ForwardedFor string          `json:"forwardedFor,omitempty"`
}

// Fault makes the next Count requests matching Method (and Name, when set)
// fail with Status.
type Fault struct {
	Method string `json:"method"`
	Name   string `json:"name,omitempty"`
	Count  int    `json:"count"`
	Status int    `json:"status"`
}

type Server struct {
	mu     sync.Mutex
	things map[string]Thing
	calls  []Call
	faults []Fault
}

func NewServer() *Server {
	return &Server{things: map[string]Thing{}}
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	defer s.mu.Unlock()

	switch {
	case r.URL.Path == "/_calls":
		s.serveCalls(w, r)
	case r.URL.Path == "/_faults":
		s.serveFaults(w, r)
	case r.URL.Path == "/healthz":
		w.WriteHeader(http.StatusOK)
	case strings.HasPrefix(r.URL.Path, "/things/"):
		s.serveThing(w, r)
	default:
		http.NotFound(w, r)
	}
}

func (s *Server) serveCalls(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		w.Header().Set("Content-Type", "application/x-ndjson")
		enc := json.NewEncoder(w)
		for _, c := range s.calls {
			_ = enc.Encode(c)
		}
	case http.MethodDelete:
		s.calls = nil
		w.WriteHeader(http.StatusNoContent)
	default:
		w.WriteHeader(http.StatusMethodNotAllowed)
	}
}

func (s *Server) serveFaults(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		writeJSON(w, http.StatusOK, s.faults)
	case http.MethodPost:
		var f Fault
		if err := json.NewDecoder(r.Body).Decode(&f); err != nil || f.Method == "" || f.Count <= 0 || f.Status < 400 {
			http.Error(w, "want {method, count > 0, status >= 400, name?}", http.StatusBadRequest)
			return
		}
		s.faults = append(s.faults, f)
		w.WriteHeader(http.StatusNoContent)
	case http.MethodDelete:
		s.faults = nil
		w.WriteHeader(http.StatusNoContent)
	default:
		w.WriteHeader(http.StatusMethodNotAllowed)
	}
}

func (s *Server) serveThing(w http.ResponseWriter, r *http.Request) {
	name := strings.TrimPrefix(r.URL.Path, "/things/")
	body, _ := io.ReadAll(r.Body)
	status, resp, fault := s.handleThing(r.Method, name, body)

	c := Call{
		Seq: len(s.calls) + 1, Method: r.Method, Path: r.URL.Path, Status: status, Fault: fault,
		UserAgent: r.UserAgent(), ForwardedFor: r.Header.Get("X-Forwarded-For"),
	}
	if compact := new(bytes.Buffer); len(body) > 0 && json.Compact(compact, body) == nil {
		c.Body = compact.Bytes()
	}
	s.calls = append(s.calls, c)

	if resp == nil {
		w.WriteHeader(status)
		return
	}
	writeJSON(w, status, resp)
}

func (s *Server) handleThing(method, name string, body []byte) (int, any, bool) {
	if name == "" || strings.Contains(name, "/") {
		return http.StatusNotFound, nil, false
	}
	if status, ok := s.takeFault(method, name); ok {
		return status, map[string]string{"error": "injected fault"}, true
	}
	status, resp := s.thing(method, name, body)
	return status, resp, false
}

func (s *Server) thing(method, name string, body []byte) (int, any) {
	switch method {
	case http.MethodGet:
		t, ok := s.things[name]
		if !ok {
			return http.StatusNotFound, nil
		}
		return http.StatusOK, t
	case http.MethodPut:
		var t Thing
		if err := json.Unmarshal(body, &t); err != nil {
			return http.StatusBadRequest, map[string]string{"error": "body must be {\"configurableField\": string}"}
		}
		_, existed := s.things[name]
		s.things[name] = t
		if existed {
			return http.StatusOK, t
		}
		return http.StatusCreated, t
	case http.MethodDelete:
		if _, ok := s.things[name]; !ok {
			return http.StatusNotFound, nil
		}
		delete(s.things, name)
		return http.StatusNoContent, nil
	default:
		return http.StatusMethodNotAllowed, nil
	}
}

func (s *Server) takeFault(method, name string) (int, bool) {
	for i := range s.faults {
		f := &s.faults[i]
		if f.Method != method || (f.Name != "" && f.Name != name) {
			continue
		}
		f.Count--
		status := f.Status
		if f.Count == 0 {
			s.faults = append(s.faults[:i], s.faults[i+1:]...)
		}
		return status, true
	}
	return 0, false
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

func main() {
	addr := os.Getenv("LISTEN_ADDR")
	if addr == "" {
		addr = ":8080"
	}
	log.Printf("fake-external listening on %s", addr)
	log.Fatal(http.ListenAndServe(addr, NewServer()))
}
