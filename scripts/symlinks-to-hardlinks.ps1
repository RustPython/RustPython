#!/usr/bin/env powershell

Set-Location -Path $(Split-Path -Parent $PSScriptRoot)

git ls-files -s | Select-String '^120000' | ConvertFrom-String -PropertyNames Get-Content,Hash,_,Path | ForEach-Object {
  $symlink = $_.Path

  git checkout --quiet -- $symlink
  if (Test-Path $symlink -PathType Leaf) {
    $parent = (Get-Item $symlink).Directory
  } else {
    $parent = (Get-Item $symlink).Parent
  }
  $child = (Get-Content $symlink)

  $src = (Join-Path -Path $parent -ChildPath $child)

  if (Test-Path $src -PathType Leaf) {
    Remove-Item $symlink
    New-Item -ItemType HardLink -Name $symlink -Value $src
  } elseif (Test-Path $src -PathType Container) {
    Remove-Item $symlink
    New-Item -ItemType Junction -Name $symlink -Value $src
  } else {
    Write-Error 'error: git-rm-symlink: Not a valid source\n'
    Write-Error '$symlink =/=> $src...'
    return
  }

  git update-index --assume-unchanged $symlink
}
