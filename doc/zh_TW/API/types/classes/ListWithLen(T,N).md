# ListWithLen T: Type, N: Nat

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/ListWithLen(T,N).md%26commit_hash%3D13f2d31aee9012f60b7a40d4b764921f1419cdfe)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/ListWithLen(T,N).md&commit_hash=13f2d31aee9012f60b7a40d4b764921f1419cdfe)

`[T; N]`是語法糖。還有一個[`List` 類型](../../../syntax/10_list.md)省略了長度

## 方法

* values_at(self, selectors: [Nat; N]) -> [T; N]

```python
assert ["a", "b", "c", "d", "e"].values_at([0, 1, 3]) == ["a", "b", "d"]
```

* all(self, pred: T -> Bool) -> Bool
  返回是否所有元素都滿足 pred
   如果元素為 0，則無論 pred 為 `True`，但會發出警告
   該規范本身已被多種語言采用，是邏輯一致性所必需的

  ```python
  assert [].all(_ -> False)
  ```

  ```python
  assert all(False for _ in [])
  ```

## ListWithLen T, N | T <: Eq 的方法

* freq self -> [{T: Nat}]
  返回對象出現的次數

```python
assert ["a", "b", "c", "b", "c", "b"].freq() \
== [{"a", 1}, {"b": 3}, {"c": 2}]
```
