package provider

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"time"
)

// Thing is the external resource behind a MyType.
type Thing struct {
	ConfigurableField string `json:"configurableField"`
}

// Client talks to the parity fake external service. Its error texts are part
// of the parity contract (parity/README.md), so the Rust provider uses the same.
type Client struct {
	endpoint string
	http     *http.Client
}

func NewClient(_ context.Context, _ ClientConfig) (*Client, error) {
	return &Client{endpoint: *Endpoint, http: &http.Client{Timeout: 10 * time.Second}}, nil
}

// Get returns nil and no error when the thing does not exist.
func (c *Client) Get(ctx context.Context, name string) (*Thing, error) {
	code, t, err := c.do(ctx, http.MethodGet, name, nil)
	switch {
	case err != nil:
		return nil, err
	case code == http.StatusNotFound:
		return nil, nil
	case code == http.StatusOK:
		return t, nil
	}
	return nil, statusError(http.MethodGet, name, code)
}

func (c *Client) Put(ctx context.Context, name string, t Thing) (*Thing, error) {
	code, out, err := c.do(ctx, http.MethodPut, name, &t)
	switch {
	case err != nil:
		return nil, err
	case code == http.StatusOK || code == http.StatusCreated:
		return out, nil
	}
	return nil, statusError(http.MethodPut, name, code)
}

// Delete treats a thing that is already gone as deleted.
func (c *Client) Delete(ctx context.Context, name string) error {
	code, _, err := c.do(ctx, http.MethodDelete, name, nil)
	switch {
	case err != nil:
		return err
	case code == http.StatusNoContent || code == http.StatusNotFound:
		return nil
	}
	return statusError(http.MethodDelete, name, code)
}

func statusError(method, name string, code int) error {
	return fmt.Errorf("%s /things/%s: status %d", method, name, code)
}

func (c *Client) do(ctx context.Context, method, name string, in *Thing) (int, *Thing, error) {
	var body bytes.Buffer
	if in != nil {
		if err := json.NewEncoder(&body).Encode(in); err != nil {
			return 0, nil, err
		}
	}
	req, err := http.NewRequestWithContext(ctx, method, c.endpoint+"/things/"+url.PathEscape(name), &body)
	if err != nil {
		return 0, nil, err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return 0, nil, err
	}
	defer resp.Body.Close() //nolint:errcheck // nothing to do on a close error
	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		return resp.StatusCode, nil, nil
	}
	out := &Thing{}
	if err := json.NewDecoder(resp.Body).Decode(out); err != nil {
		return 0, nil, fmt.Errorf("%s /things/%s: cannot decode response: %w", method, name, err)
	}
	return resp.StatusCode, out, nil
}
