package server

import (
	"context"
	"net/http"
)

// MaxBody is the largest request body.
const MaxBody = 1 << 20

const (
	ModeA Mode = iota
	ModeB
)

var (
	defaultServer *Server
)

// Server serves requests.
type Server struct {
	addr string
	mux  *http.ServeMux
}

// Handler handles one request.
type Handler interface {
	Serve(ctx context.Context, r *http.Request) error
	io.Closer
}

type Mode int

type (
	Option func(*Server)
	List[T any] struct {
		items []T
	}
)

// New makes a server.
func New(addr string, opts ...Option) *Server {
	s := &Server{addr: addr}
	f := func() { _ = "func Fake() {" }
	return s
}

// Start starts it.
func (s *Server) Start(ctx context.Context) error {
	return nil
}

func (l List[T]) Len() int { return len(l.items) }

func Map[T, U any](in []T, f func(T) U) []U {
	return nil
}

var raw = `
func NotReal() {
`
