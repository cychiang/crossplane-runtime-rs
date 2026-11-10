package main

import (
	"fmt"
	"sort"
	"time"

	"sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue"
)

func main() {
	q := priorityqueue.New[string]("probe")
	var over []float64
	for i := 0; i < 30; i++ {
		time.Sleep(time.Duration(137*(i%7)+50) * time.Microsecond)
		d := 50*time.Millisecond + time.Duration((i*9973)%1000)*time.Microsecond
		at := time.Now().Add(d)
		q.AddAfter("a", d)
		it, _ := q.Get()
		over = append(over, float64(time.Since(at).Microseconds())/1000)
		q.Done(it)
	}
	sort.Float64s(over)
	s := 0.0
	for _, o := range over {
		s += o
	}
	fmt.Printf("go priorityqueue AddAfter+Get: n=%d mean=%.3fms median=%.3fms\n", len(over), s/float64(len(over)), over[len(over)/2])
}
