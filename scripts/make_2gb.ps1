# 약 2GB CSV 생성 (병렬 로딩 체감용). 행당 ~90바이트.
$path = "big_2gb.csv"
$rows = 22000000
$sw = [System.IO.StreamWriter]::new($path)
$sw.WriteLine("id,name,city,status,date,amount,dept")
for ($i = 1; $i -le $rows; $i++) {
    $sw.WriteLine("$i,John Smith,New York City NY,active,2026-01-15,4321.99,department-engineering-team")
}
$sw.Close()
Write-Host "created $path ($((Get-Item $path).Length / 1GB) GB)"
