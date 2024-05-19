# `erg` 構建功能

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/dev_guide/build_features.md%26commit_hash%3Dddb483c2cf733dba776fd6a5589f28871a2c3e62)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/dev_guide/build_features.md&commit_hash=ddb483c2cf733dba776fd6a5589f28871a2c3e62)

## debug

進入調試模式。結果，Erg 內部的行為順序顯示在日誌中
獨立於 Rust 的 `debug_assertions` 標誌.
Also, enable `backtrace_on_stack_overflow`.

## backtrace

Enable only `backtrace_on_stack_overflow`.

## japanese

將系統語言設置為日語
Erg 內部選項、幫助(幫助、版權、許可證等)和錯誤顯示為日語

## simplified_chinese

將系統語言設置為簡體中文
Erg 內部選項、幫助(幫助、版權、許可證等)和錯誤顯示為簡體中文

## traditional_chinese

將系統語言設置為繁體中文
Erg 內部選項、幫助(幫助、版權、許可證等)和錯誤顯示為繁體中文。

## unicode/pretty

使得編譯器顯示豐富內容

## large_thread

增加線程堆棧大小。用于Windows執行和測試執行

## els

通過 `--language-server` 使其變得可用
通過 `erg --language-server` 打開

## py_compatible

啟用Python兼容模式，使部分api和語法與Python兼容。用于[pylyzer](https://github.com/mtshiba/pylyzer)
