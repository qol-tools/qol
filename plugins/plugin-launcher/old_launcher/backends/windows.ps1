param([string]$query)

if ([string]::IsNullOrEmpty($query)) {
    exit 0
}

$limit = 50

# Try Everything CLI first (faster, if installed)
$es = Get-Command "es.exe" -ErrorAction SilentlyContinue
if ($es) {
    & es.exe -n $limit -i $query 2>$null
    exit 0
}

# Fallback to Windows Search
$conn = New-Object -ComObject ADODB.Connection
$rs = New-Object -ComObject ADODB.Recordset

try {
    $conn.Open("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';")
    $sql = "SELECT TOP $limit System.ItemPathDisplay FROM SystemIndex WHERE System.FileName LIKE '%$query%'"
    $rs.Open($sql, $conn)

    while (-not $rs.EOF) {
        Write-Output $rs.Fields.Item("System.ItemPathDisplay").Value
        $rs.MoveNext()
    }
} finally {
    if ($rs.State -eq 1) { $rs.Close() }
    if ($conn.State -eq 1) { $conn.Close() }
}
