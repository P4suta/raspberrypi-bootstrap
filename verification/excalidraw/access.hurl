GET {{origin}}/
HTTP 403

GET {{origin}}/
Tailscale-User-Login: somebody-else@example.invalid
HTTP 403

GET {{origin}}/
Tailscale-User-Login: {{reader}}
HTTP 200
[Asserts]
header "Content-Type" contains "text/html"
body contains "Excalidraw"
body not contains "https://excalidraw.nyc3.cdn.digitaloceanspaces.com/oss/"
body not contains "document.body.appendChild(scriptEle);"
header "Content-Security-Policy" contains "font-src 'self'"

GET {{origin}}/fonts/Assistant/Assistant-Regular.woff2
Tailscale-User-Login: {{reader}}
HTTP 200
[Asserts]
header "Content-Type" contains "woff2"

GET {{origin}}/diagrams/
Tailscale-User-Login: {{reader}}
HTTP 404

GET {{origin}}/diagrams/00000000000000000000000000000001/scene.excalidraw
Tailscale-User-Login: {{reader}}
HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.type" == "excalidraw"

GET {{origin}}/diagrams/00000000000000000000000000000001/unpublished.json
Tailscale-User-Login: {{reader}}
HTTP 404
