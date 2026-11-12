GET {{site_origin}}/
HTTP 308
[Asserts]
header "Location" == "/study/"

GET {{site_origin}}/study
HTTP 308
[Asserts]
header "Location" == "/study/"

GET {{site_origin}}/missing-family-service
HTTP 404

GET {{site_origin}}/.env
HTTP 404
