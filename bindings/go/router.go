//go:build ariarouter_ffi

package ariarouter

/*
#cgo CFLAGS: -I${SRCDIR}/../../ffi/include
#cgo LDFLAGS: -L${SRCDIR}/../../target/debug -L${SRCDIR}/../../target/release -lariarouter_ffi
#include "ariarouter.h"
#include <stdlib.h>
*/
import "C"
import (
	"encoding/json"
	"errors"
	"os"
	"unsafe"
)

type Router struct {
	h            unsafe.Pointer
	authBaseURL  string
	authToken    string
}

func NewRouter() *Router { return &Router{} }

func (r *Router) Setup(baseURL, token string) {
	if baseURL != "" {
		r.authBaseURL = baseURL
	}
	if token != "" {
		r.authToken = token
	}
}

func (r *Router) SetupClear() {
	r.authBaseURL, r.authToken = "", ""
}

func (r *Router) Init(configPath string) error {
	cs := C.CString(configPath)
	defer C.free(unsafe.Pointer(cs))
	h := C.ariarouter_init(cs)
	if h == nil {
		return errors.New(C.GoString(C.ariarouter_last_error()))
	}
	r.Close()
	r.h = unsafe.Pointer(h)
	return nil
}

func (r *Router) Close() {
	if r.h != nil {
		C.ariarouter_destroy((*C.Ariarouter)(r.h))
		r.h = nil
	}
}

func (r *Router) native() *C.Ariarouter { return (*C.Ariarouter)(r.h) }

func (r *Router) Complete(messages any, options any) (map[string]any, error) {
	mb, _ := json.Marshal(messages)
	ob, _ := json.Marshal(options)
	ms, os_ := C.CString(string(mb)), C.CString(string(ob))
	defer C.free(unsafe.Pointer(ms))
	defer C.free(unsafe.Pointer(os_))
	buf := make([]byte, 256*1024)
	rc := C.ariarouter_complete(r.native(), ms, os_, (*C.char)(unsafe.Pointer(&buf[0])), C.size_t(len(buf)))
	if rc != 0 {
		return nil, errors.New(C.GoString(C.ariarouter_last_error()))
	}
	var out map[string]any
	n := 0
	for n < len(buf) && buf[n] != 0 {
		n++
	}
	if err := json.Unmarshal(buf[:n], &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (r *Router) Models() (map[string]any, error) {
	buf := make([]byte, 64*1024)
	rc := C.ariarouter_models(r.native(), (*C.char)(unsafe.Pointer(&buf[0])), C.size_t(len(buf)))
	if rc != 0 {
		return nil, errors.New(C.GoString(C.ariarouter_last_error()))
	}
	n := 0
	for n < len(buf) && buf[n] != 0 {
		n++
	}
	var out map[string]any
	if err := json.Unmarshal(buf[:n], &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (r *Router) LastRoute() (map[string]any, error) {
	buf := make([]byte, 64*1024)
	rc := C.ariarouter_last_route(r.native(), (*C.char)(unsafe.Pointer(&buf[0])), C.size_t(len(buf)))
	if rc != 0 {
		return nil, errors.New(C.GoString(C.ariarouter_last_error()))
	}
	n := 0
	for n < len(buf) && buf[n] != 0 {
		n++
	}
	var out map[string]any
	if err := json.Unmarshal(buf[:n], &out); err != nil {
		return nil, err
	}
	return out, nil
}

func ConfigEnv() string { return os.Getenv("ARIAROUTER_CONFIG") }
