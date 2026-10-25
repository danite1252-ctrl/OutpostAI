<#
  Pretend to be a phone and pair with this Outpost over its encrypted port, the way
  the phone app will. Run it on this PC or another one on the same network.

    1. In Outpost: Connections > Pair a phone (leave the dialog open).
    2. powershell -ExecutionPolicy Bypass -File scripts\phone\pair-test.ps1
       (add -Server 192.168.1.20 when running on another machine)

  It checks the certificate's three words against the dialog, pairs with your code and
  PIN, then shows the phone token works on the encrypted port and is refused over plain
  HTTP. Unpair "PowerShell test" in Connections afterwards.
#>
param(
    [string]$Server = "",
    [int]$Port = 7421,
    [int]$ApiPort = 7420
)
$ErrorActionPreference = "Stop"

if (-not $Server) {
    $Server = (Get-NetIPAddress -AddressFamily IPv4 |
        Where-Object { $_.IPAddress -notlike "127.*" -and $_.IPAddress -notlike "169.254.*" } |
        Sort-Object -Property InterfaceMetric | Select-Object -First 1).IPAddress
}
Write-Host "Outpost at $Server, encrypted port $Port" -ForegroundColor Cyan

# 1. Read the certificate the server presents, as a phone would.
$tcp = New-Object Net.Sockets.TcpClient($Server, $Port)
$ssl = New-Object Net.Security.SslStream($tcp.GetStream(), $false, ({ $true }))
$ssl.AuthenticateAsClient("outpost.local")
$der = $ssl.RemoteCertificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert)
$ssl.Dispose(); $tcp.Dispose()
$sha = [Security.Cryptography.SHA256]::Create()
$fp = -join ($sha.ComputeHash($der) | ForEach-Object { $_.ToString("x2") })

$words = @('acorn','adobe','agent','album','alley','amber','anchor','angle','apple','apron','arrow','aspen','atlas','attic','autumn','badge','bagel','baker','bamboo','banjo','barn','basil','beach','beacon','bean','bear','bell','berry','bike','birch','bison','blade','blanket','bloom','boat','bonus','boot','bottle','brick','bridge','brook','brush','bucket','buffalo','bugle','cabin','cactus','camel','candle','canoe','canyon','carpet','castle','cedar','cello','chalk','cherry','chess','cider','cinema','circle','clock','cloud','clover','coast','cobalt','comet','copper','coral','cotton','cowboy','crane','crayon','cricket','crown','cubby','daisy','delta','denim','desert','diamond','dingo','dolphin','domino','donkey','dragon','drum','dune','eagle','easel','echo','elbow','ember','emerald','engine','falcon','feather','fern','fiddle','fig','flame','flute','forest','fossil','fox','frost','galaxy','garden','garlic','gecko','ginger','glacier','globe','goose','granite','grape','gravel','guitar','hammer','harbor','harp','hazel','helmet','heron','hickory','honey','horizon','husky','igloo','island','ivory','jacket','jaguar','jasmine','jelly','jewel','jigsaw','jungle','kayak','kettle','kiwi','koala','ladder','lagoon','lantern','laser','lemon','lily','lime','lizard','lobster','locket','lotus','magnet','mango','maple','marble','meadow','melon','meteor','mint','mirror','mitten','moose','mosaic','muffin','nectar','needle','nickel','noodle','nutmeg','oasis','ocean','olive','onion','opal','orbit','orchid','otter','owl','paddle','panda','parrot','peach','pebble','pencil','pepper','piano','pickle','pigeon','pillow','pine','planet','plum','pocket','pony','poppy','prism','puffin','pumpkin','quartz','quilt','rabbit','radar','raven','reef','ribbon','river','robin','rocket','rose','ruby','saddle','salmon','sandal','satin','seal','shell','silver','sketch','sled','snail','sparrow','spider','spoon','spruce','squid','stone','storm','sugar','summit','sunset','swan','tango','teapot','thistle','thunder','tiger','timber','toast','tomato','topaz','torch','tulip','tundra','turtle','umbrella','valley','velvet','violet','walnut','walrus','willow','window','wizard','yak')
$h = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes("outpost-words:$fp"))
$check = "$($words[$h[0]]) - $($words[$h[1]]) - $($words[$h[2]])"
Write-Host ""
Write-Host "Check words: $check" -ForegroundColor Yellow
Write-Host "Fingerprint: $fp"
if ((Read-Host "Same three words as the Pair a phone dialog? (y/n)") -ne "y") {
    Write-Host "Stopped: this isn't your Outpost, or the network is being tampered with." -ForegroundColor Red
    exit 1
}

# 2. From here on, accept only that certificate (pinning).
$pinned = $fp
if ($PSVersionTable.PSVersion.Major -ge 6) {
    $tls = @{ SkipCertificateCheck = $true }   # checked by hand above
} else {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Add-Type @"
using System.Net; using System.Security.Cryptography; using System.Security.Cryptography.X509Certificates;
public static class OutpostPin {
    public static string Fingerprint;
    public static bool Check(object s, X509Certificate c, X509Chain ch, System.Net.Security.SslPolicyErrors e) {
        var h = SHA256.Create().ComputeHash(c.GetRawCertData());
        var sb = new System.Text.StringBuilder(); foreach (var b in h) sb.Append(b.ToString("x2"));
        return sb.ToString() == Fingerprint;
    }
    public static System.Net.Security.RemoteCertificateValidationCallback Callback { get { return Check; } }
}
"@
    [OutpostPin]::Fingerprint = $pinned
    [Net.ServicePointManager]::ServerCertificateValidationCallback = [OutpostPin]::Callback
    $tls = @{}
}
$base = "https://${Server}:$Port"
function Post($path, $body) {
    try {
        Invoke-RestMethod -Method Post -Uri "$base$path" -ContentType "application/json" -Body ($body | ConvertTo-Json) @tls
    } catch {
        # Show Outpost's own message, the way the phone app will.
        $msg = $_.ErrorDetails.Message
        try { $msg = ($msg | ConvertFrom-Json).error.message } catch {}
        if (-not $msg) { $msg = $_.Exception.Message }
        Write-Host "Outpost said: $msg" -ForegroundColor Red
        exit 1
    }
}

# 3. The code, then an account and its PIN.
$code = (Read-Host "Code shown on the PC").Replace(" ", "")
$hello = Post "/api/pair/hello" @{ code = $code }
Write-Host "Reached $($hello.name) (Outpost $($hello.version))" -ForegroundColor Green
for ($i = 0; $i -lt $hello.accounts.Count; $i++) { Write-Host "  [$i] $($hello.accounts[$i].name)" }
$pick = $hello.accounts[[int](Read-Host "Which account")]
$secure = Read-Host "PIN for $($pick.name)" -AsSecureString
$pin = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
$paired = Post "/api/pair/complete" @{ code = $code; account_id = $pick.id; secret = $pin; device_name = "PowerShell test" }
Write-Host "Paired as $($paired.account.name); the Pair a phone dialog should now say so." -ForegroundColor Green

# 4. The phone token works on the encrypted port...
$auth = @{ Authorization = "Bearer $($paired.token)" }
$me = Invoke-RestMethod -Uri "$base/api/me" -Headers $auth @tls
Write-Host "Encrypted port: signed in as $($me.account.name) on '$($me.session.label)'" -ForegroundColor Green

# ...and is refused over plain HTTP from the network.
try {
    Invoke-RestMethod -Uri "http://${Server}:$ApiPort/api/me" -Headers $auth | Out-Null
    Write-Host "Plain HTTP accepted the phone token (expected only when run on the Outpost PC itself)." -ForegroundColor Yellow
} catch {
    Write-Host "Plain HTTP refused the phone token, as it should." -ForegroundColor Green
}
Write-Host ""
Write-Host "Done. Unpair 'PowerShell test' in Connections > Phones."
