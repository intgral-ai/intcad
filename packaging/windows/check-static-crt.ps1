param([Parameter(Mandatory = $true)][string] $Path)

$ErrorActionPreference = 'Stop'

function Assert-StaticCrt($path) {
    # The Windows package claims the C runtime is linked statically, so no VC++
    # redistributable is needed. This check is what makes that claim true: a
    # build whose +crt-static silently failed imports one of these DLLs and
    # would fail on a clean machine. Only the PE headers and the import
    # directory are read, never the whole image.
    $banned = @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')
    $stream = [IO.File]::OpenRead($path)
    try {
        $reader = [IO.BinaryReader]::new($stream)
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) { throw 'Missing DOS header' }
        $stream.Position = 0x3c
        $pe = [long]$reader.ReadUInt32()
        if ($pe -lt 64 -or $pe -gt ($stream.Length - 24)) { throw 'PE header outside the file' }
        $stream.Position = $pe
        if ($reader.ReadUInt32() -ne 0x4550) { throw 'Missing PE signature' }
        $null = $reader.ReadUInt16()
        $sections = $reader.ReadUInt16()
        $stream.Position += 12
        $optionalSize = $reader.ReadUInt16()
        $null = $reader.ReadUInt16()
        $optStart = $stream.Position
        if ($reader.ReadUInt16() -ne 0x020b) { throw 'Not a PE32+ image' }
        $stream.Position = $optStart + 112
        $null = $reader.ReadUInt32()
        $null = $reader.ReadUInt32()
        $importRva = $reader.ReadUInt32()
        $importSize = $reader.ReadUInt32()
        if ($importRva -eq 0 -or $importSize -eq 0) { return }
        $stream.Position = $optStart + $optionalSize
        $map = @()
        for ($i = 0; $i -lt $sections; $i++) {
            $null = $reader.ReadBytes(8)
            $vsize = $reader.ReadUInt32()
            $vaddr = $reader.ReadUInt32()
            $rawSize = $reader.ReadUInt32()
            $rawPtr = $reader.ReadUInt32()
            $null = $reader.ReadBytes(16)
            $map += [pscustomobject]@{ VAddr = $vaddr; VSize = $vsize; RawPtr = $rawPtr; RawSize = $rawSize }
        }
        function Convert-Rva([long] $rva) {
            foreach ($s in $map) {
                $size = if ($s.VSize -gt 0) { $s.VSize } else { $s.RawSize }
                if ($rva -ge $s.VAddr -and $rva -lt ($s.VAddr + $size)) {
                    return [long]($s.RawPtr + ($rva - $s.VAddr))
                }
            }
            throw 'Import directory is outside every section'
        }
        $walk = Convert-Rva $importRva
        for ($entry = 0; $entry -lt 4096; $entry++) {
            $stream.Position = $walk + ($entry * 20)
            $null = $reader.ReadUInt32()
            $null = $reader.ReadUInt32()
            $null = $reader.ReadUInt32()
            $nameRva = $reader.ReadUInt32()
            $null = $reader.ReadUInt32()
            if ($nameRva -eq 0) { break }
            $stream.Position = Convert-Rva $nameRva
            $chars = [Collections.Generic.List[byte]]::new()
            while ($chars.Count -lt 256) {
                $b = $reader.ReadByte()
                if ($b -eq 0) { break }
                $chars.Add($b)
            }
            $name = [Text.Encoding]::ASCII.GetString($chars.ToArray())
            if ($banned -contains $name.ToLowerInvariant()) {
                throw "$([IO.Path]::GetFileName($path)) imports $name; the static C runtime is required"
            }
        }
    } finally { $stream.Dispose() }
}

Assert-StaticCrt $Path
Write-Output "ok $([IO.Path]::GetFileName($Path)): static C runtime"
