# 合成大型CSV生成脚本
# 约 ~500K 行 ≈ 12-15MB CSV (快速烟测)
# 对于完整1GB+/10GB+测试，将 $rows 改为：
#   - 1GB: 40000000 行
#   - 10GB: 400000000 行
# 或设置 $rows 为任意需要的行数。

$path = "big_test.csv"
$rows = 500000

$sw = [System.IO.StreamWriter]::new($path)
$sw.WriteLine("id,name,value,city")
for ($i = 1; $i -le $rows; $i++) {
    $sw.WriteLine("$i,name$i,$($i * 3),city$($i % 100)")
}
$sw.Close()
Write-Host "created $path"
