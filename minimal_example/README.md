This directory contains a minimal dropshot server example. Run with:

```
$ cargo run -p minimal_example
   Compiling minimal_example v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in 2.52s
     Running `target/debug/minimal_example`
Jan 07 13:57:09.848 INFO Server uuid is cc33de2e-2609-48d4-94ba-5929688a4416
Jan 07 13:57:09.848 DEBG registered endpoint, path: /hello, method: GET, local_addr: 127.0.0.1:4567
Jan 07 13:57:09.848 INFO listening, local_addr: 127.0.0.1:4567
Jan 07 13:57:09.848 DEBG DTrace USDT probes compiled out, not registering, local_addr: 127.0.0.1:4567
```

It will expose a single endpoint at /hello:

```
$ curl -sq http://127.0.0.1:4567/hello | jq .
{
  "message": "Hello from dropshot!",
  "uuid": "cc33de2e-2609-48d4-94ba-5929688a4416"
}
```

Note that the request was logged:

```
Jan 07 13:57:23.969 INFO accepted connection, remote_addr: 127.0.0.1:45634, local_addr: 127.0.0.1:4567
Jan 07 13:57:23.969 TRCE incoming request, uri: /hello, method: GET, req_id: ab18568d-d22f-40bc-8ef4-3b8bb1fe0738, remote_addr: 127.0.0.1:45634, local_addr: 127.0.0.1:4567
Jan 07 13:57:23.969 INFO request completed, response_code: 200, uri: /hello, method: GET, req_id: ab18568d-d22f-40bc-8ef4-3b8bb1fe0738, remote_addr: 127.0.0.1:45634, local_addr: 127.0.0.1:4567
```
