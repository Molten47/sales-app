$base = "http://localhost:8081"; $json = "application/json"

function Login($email) {
    Invoke-RestMethod -Method Post -Uri "$base/api/v1/auth/request-otp" -ContentType $json -Body (@{ email=$email } | ConvertTo-Json) | Out-Null
    $code = Read-Host "OTP code from the app log for $email"
    Invoke-RestMethod -Method Post -Uri "$base/api/v1/auth/verify-otp" -ContentType $json -Body (@{ email=$email; code=$code } | ConvertTo-Json)
}
function Status($script) { try { & $script | Out-Null; "200/2xx" } catch { $_.Exception.Response.StatusCode.value__ } }
