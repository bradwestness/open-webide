# API examples

```sh
# Sign in (register the first account in the UI first). Keep this cookie jar private.
curl -s -c /tmp/openwebide-cookies -X POST localhost:3000/api/login \
  -H 'x-openwebide: 1' -H 'content-type: application/json' \
  -d '{"username":"your-user","password":"your-password"}'

# add a connection
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X POST localhost:3000/api/connections \
  -H 'content-type: application/json' \
  -d '{"name":"local-ollama","kind":"ollama","base_url":"http://localhost:11434","model":"qwen2.5-coder:7b"}'

curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' localhost:3000/api/connections

# settings
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X PUT localhost:3000/api/settings \
  -H 'content-type: application/json' \
  -d '{"key":"theme","value":"dark"}'
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' localhost:3000/api/settings

# system prompts
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X POST localhost:3000/api/system-prompts \
  -H 'content-type: application/json' \
  -d '{"name":"coder","content":"You are a coding agent."}'

# provider endpoints (need a running engine at the connection's base_url)
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' 'localhost:3000/api/models?connection_id=1'

curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X POST localhost:3000/api/chat \
  -H 'content-type: application/json' \
  -d '{"connection_id":1,"messages":[{"role":"user","content":"hello"}]}'
```


[Back to the README](../README.md).
