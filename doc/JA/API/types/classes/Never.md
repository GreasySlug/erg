# Never

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Never.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Never.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

全ての型の下位型である。全てのメソッドを持っており、当然`.new`も持っているため`Class`である。しかしインスタンスは持たず、生成されそうになった瞬間にErgは停止する。
`Panic`という同じくインスタンスを持たない型が存在するが、正常に終了する際や意図的な無限ループの際は`Never`、異常終了する際には`Panic`を使う。

```python,checker_ignore
# Never <: Panic
f(): Panic = exit 0 # OK
g(): Panic = panic "..." # OK

may_fail(): Int or Panic = ...
h(): Never = may_fail() # TypeError: `Panic`は`Never`ではない
```

`panic`・`todo`・`unreachable`は`Panic`ではなく`Never`を返す。これは`f(): Int = todo()`のようなスタブが型検査を通るようにするためである。

`T or Never`は`T`である。`Never`は意味論上起こり得ない(起こった場合プログラムは即時停止する)選択肢であるため、Or型を作った時点で吸収される。
`T or Panic`は吸収されない。プログラムの終了が起こりうることを読み手に伝えるためシグネチャに残る。ただし同じ理由により、その型の値は`T`として使用できる。[Panic](./Panic.md)を参照。
