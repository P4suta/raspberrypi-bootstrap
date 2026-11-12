GET {{site_url}}/
HTTP 200
[Captures]
script: xpath "string(//script[@type='module']/@src)"
style: xpath "string(//link[@rel='stylesheet']/@href)"
[Asserts]
header "Content-Type" contains "text/html"
body contains "Maintenance study player"

GET {{site_url}}/{{script}}
HTTP 200
[Asserts]
header "Content-Type" contains "javascript"
body contains "WebAssembly"

GET {{site_url}}/{{style}}
HTTP 200
[Asserts]
header "Content-Type" contains "text/css"

GET {{site_url}}/favicon.svg
HTTP 200
[Asserts]
header "Content-Type" contains "image/svg+xml"

GET {{site_url}}/.env
HTTP 404
