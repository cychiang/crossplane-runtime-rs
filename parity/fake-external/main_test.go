package main

import (
	"bufio"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
)

type client struct {
	t   *testing.T
	url string
}

func (c client) do(method, path, body string) (int, string) {
	c.t.Helper()
	req, err := http.NewRequest(method, c.url+path, strings.NewReader(body))
	if err != nil {
		c.t.Fatal(err)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		c.t.Fatal(err)
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, strings.TrimSpace(string(b))
}

func (c client) calls() []Call {
	c.t.Helper()
	_, body := c.do(http.MethodGet, "/_calls", "")
	var out []Call
	sc := bufio.NewScanner(strings.NewReader(body))
	for sc.Scan() {
		var call Call
		if err := json.Unmarshal(sc.Bytes(), &call); err != nil {
			c.t.Fatalf("bad call line %q: %v", sc.Text(), err)
		}
		out = append(out, call)
	}
	return out
}

func newClient(t *testing.T) client {
	srv := httptest.NewServer(NewServer())
	t.Cleanup(srv.Close)
	return client{t: t, url: srv.URL}
}

func TestThingLifecycle(t *testing.T) {
	c := newClient(t)

	steps := []struct {
		method, path, body string
		wantStatus         int
		wantBody           string
	}{
		{http.MethodGet, "/things/a", "", 404, ""},
		{http.MethodPut, "/things/a", `{"configurableField": "one"}`, 201, `{"configurableField":"one"}`},
		{http.MethodGet, "/things/a", "", 200, `{"configurableField":"one"}`},
		{http.MethodPut, "/things/a", `{"configurableField":"two"}`, 200, `{"configurableField":"two"}`},
		{http.MethodGet, "/things/a", "", 200, `{"configurableField":"two"}`},
		{http.MethodPut, "/things/a", `not json`, 400, `{"error":"body must be {\"configurableField\": string}"}`},
		{http.MethodDelete, "/things/a", "", 204, ""},
		{http.MethodDelete, "/things/a", "", 404, ""},
		{http.MethodGet, "/things/a", "", 404, ""},
	}
	for i, s := range steps {
		status, body := c.do(s.method, s.path, s.body)
		if status != s.wantStatus || body != s.wantBody {
			t.Fatalf("step %d %s %s: got %d %q, want %d %q", i, s.method, s.path, status, body, s.wantStatus, s.wantBody)
		}
	}
}

func TestCallLogRecordsThingRequestsInOrder(t *testing.T) {
	c := newClient(t)
	c.do(http.MethodGet, "/things/a", "")
	c.do(http.MethodPut, "/things/a", "{ \"configurableField\" : \"x\" }")
	c.do(http.MethodPost, "/_faults", `{"method":"GET","count":1,"status":500}`)
	c.do(http.MethodDelete, "/things/a", "")

	want := []Call{
		{Seq: 1, Method: "GET", Path: "/things/a", Status: 404},
		{Seq: 2, Method: "PUT", Path: "/things/a", Body: json.RawMessage(`{"configurableField":"x"}`), Status: 201},
		{Seq: 3, Method: "DELETE", Path: "/things/a", Status: 204},
	}
	if got := c.calls(); !reflect.DeepEqual(got, want) {
		t.Fatalf("calls:\n got %+v\nwant %+v", got, want)
	}

	if status, _ := c.do(http.MethodDelete, "/_calls", ""); status != 204 {
		t.Fatalf("clear calls: %d", status)
	}
	if got := c.calls(); len(got) != 0 {
		t.Fatalf("calls after clear: %+v", got)
	}
	c.do(http.MethodGet, "/things/a", "")
	if got := c.calls(); len(got) != 1 || got[0].Seq != 1 {
		t.Fatalf("sequence restarts after clear: %+v", got)
	}
}

func TestFaultsFailTheNextMatchingRequests(t *testing.T) {
	c := newClient(t)
	if status, _ := c.do(http.MethodPost, "/_faults", `{"method":"PUT","name":"a","count":2,"status":500}`); status != 204 {
		t.Fatalf("add fault: %d", status)
	}

	for i, s := range []struct {
		path       string
		wantStatus int
	}{
		{"/things/b", 201},
		{"/things/a", 500},
		{"/things/a", 500},
		{"/things/a", 201},
	} {
		if status, _ := c.do(http.MethodPut, s.path, `{"configurableField":"v"}`); status != s.wantStatus {
			t.Fatalf("put %d %s: got %d, want %d", i, s.path, status, s.wantStatus)
		}
	}

	calls := c.calls()
	if calls[1].Status != 500 || calls[1].Path != "/things/a" {
		t.Fatalf("failed request is logged with its status: %+v", calls[1])
	}
	if _, body := c.do(http.MethodGet, "/_faults", ""); body != "[]" && body != "null" {
		t.Fatalf("exhausted fault is removed: %s", body)
	}
}

func TestClearFaults(t *testing.T) {
	c := newClient(t)
	c.do(http.MethodPost, "/_faults", `{"method":"PUT","count":100,"status":503}`)
	c.do(http.MethodDelete, "/_faults", "")
	if status, _ := c.do(http.MethodPut, "/things/a", `{"configurableField":"v"}`); status != 201 {
		t.Fatalf("put after clearing faults: %d", status)
	}
}

func TestRejectsBadFault(t *testing.T) {
	c := newClient(t)
	for _, body := range []string{`{}`, `{"method":"PUT","count":0,"status":500}`, `{"method":"PUT","count":1,"status":200}`, `nope`} {
		if status, _ := c.do(http.MethodPost, "/_faults", body); status != 400 {
			t.Fatalf("fault %s: got %d, want 400", body, status)
		}
	}
}
