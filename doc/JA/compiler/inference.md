# 型推論アルゴリズム

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/compiler/inference.md%26commit_hash%3Dc6eb78a44de48735213413b2a28569fdc10466d0)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/compiler/inference.md&commit_hash=c6eb78a44de48735213413b2a28569fdc10466d0)

> __Warning__: この項は編集中であり、一部に間違いを含む可能性があります。

以下で用いる表記方法を示します。

```python
自由型変数(型、未束縛): ?T, ?U, ...
自由型変数(値、未束縛): ?a, ?b, ...
型環境(Γ): { x: T, ... }
型代入規則(S): { ?T --> T, ... }
型引数評価環境(E): { e -> e', ... }
```

以下のコードを例にして説明します。

```python
v = ![]
v.push! 1
print! v
```

Ergの型推論は、大枠としてHindley-Milner型推論アルゴリズムを用いています(が、種々の拡張が行わています)。
本質的には、Ergの型推論は以下の4つの問題に帰着されます。

* 多相関数(クラス)の呼び出し
* 多相関数(クラス)の定義
* 属性の解決
* 部分型判定

なぜなら、Ergでは`if`や`for!`といった制御構造も単なる(多相)関数であり、演算子も1引数ないし2引数の(多相)関数とみなせるからです。
単相関数の場合は、部分型判定のみで十分です。
[属性の解決](./attribute_resolution.md)と[部分型判定](./subtyping.md)については、別の項で説明します。
本項では関数の呼び出しと定義にかかる型推論の仕組みについて説明します。

具体的には以下の手順で型推論が行われます。用語の説明などは後述します。

1. 右辺値の型を推論する ([`search_callee_info`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/inquire.rs#L740))
2. 具体化する ([`instantiate`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/instantiate.rs#L549))
3. 実引数の型を代入する ([`substitute_call`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/inquire.rs#L1154))
4. 型パラメータを評価する ([`eval_t_params`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/eval.rs#L963))
5. 可変メソッドの場合、変更を伝搬させる ([`propagate`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/inquire.rs#L1097))
6. サブルーチン定義の場合、パラメータ型と戻り値型を汎化する ([`generalize_t`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/generalize.rs#L843))

7. 型変数を除去する ([`deref_tyvar`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/generalize.rs#L497))
   1. 型変数の部分型関係が成立できるか検証する ([`validate_simple_subsup`](https://github.com/erg-lang/erg/blob/3cc168182b051a1565aaa085f3a52533c4c3e650/crates/erg_compiler/context/generalize.rs#L734))

具体的な操作は以下になります。

line 1. Def{sig: v, block: ![]}
    get block type:
        get UnaryOp type:
            get List type: `['T; 0]`
            instantiate: `[?T; 0]`
            (substitute, eval are omitted)
    update: `Γ: {v: [?T; 0]!}`
    expr returns `NoneType`: OK

line 2. CallMethod{obj: v, name: push!, args: [1]}
    get obj type: `List!(?T, 0)`
        search: `Γ List!(?T, 0).push!({1})`
        get: `= List!('T ~> 'T, 'N ~> 'N+1).push!('T) => NoneType`
        instantiate: `List!(?T, ?N).push!(?T) => NoneType`
        substitute(`S: {?T --> Nat, ?N --> 0}`): `List!(Nat ~> Nat, 0 ~> 0+1).push!(Nat) => NoneType`
        eval: `List!(Nat, 0 ~> 1).push!({1}) => NoneType`
        update: `Γ: {v: [Nat; 1]!}`
    expr returns `NoneType`: OK

line 3. Call{obj: print!, args: [v]}
    get args type: `[[Nat; 1]!]`
    get obj type:
        search: `Γ print!([Nat; 1]!)`
        get: `= print!(*Obj) => NoneType`
    expr returns `NoneType`: OK

## 型変数の実装

型変数は[ty.rs](../../../crates/erg_compiler/ty/mod.rs)の`Type`にて当初以下のように表現されていました。現在は違う形で実装されていますが、本質的には同じアイデアであるため、より素朴な表現であるこの実装で考えます。
`RcCell<T>`は`Rc<RefCell<T>>`のラッパー型です。

```rust
pub enum Type {
    ...
    Var(RcCell<Option<Type>>), // 他の式の型への参照, docs/compiler/inference.md を参照
    ...
}
```

型変数は、実体型を外部の辞書に持っておき、型変数自体はそのキーのみを持つように実装できます。しかし、`RcCell`を使用した実装の方が一般に効率的だといわれています(要検証, [一応の出典](https://mobile.twitter.com/bd_gfngfn/status/1296719625086877696?s=21))。

型変数はまず`Type::Var(RcCell::new(None))`のようにして初期化されます。
この型変数が、コードを解析していく中で書き換えられ、型が決定されます。
最後まで中身がNoneのままだと、(その場では)具体的な型に決定できない型変数となります。例えば、多相関数`id x = x`の`x`の型がそうです。
このような状態の型変数を __未束縛型変数(Unbound type variable)__ と呼ぶことにします(正確な用語が不明)。対して、何らかの具体的な型が代入されているものは __連携型変数(Linked type variable)__ と呼ぶことにします。

両者はどちらも自由型変数という種類のものです(この用語は明らかに「自由変数」に因んで命名されていると考えられます)。これらは、コンパイラが推論のために使う型変数です。`id: 'T -> 'T`の`'T`などのように、プログラマが指定するタイプの型変数とは異なるため、このように特別な名前がついています。

未束縛型変数は、`?T`, `?U`のように表すことにします。型理論の文脈ではαやβが使われる場合が多いですが、入力の簡便化のためこちらを採用します。
これは一般的な議論のために採用した記法で、実際に文字列の識別子を使って実装されているわけではないので注意してください。

未束縛型変数`Type::Var`は、型環境に入れられる際`Type::MonoQuantVar`へと置き換えられます。これは __量化型変数(quantified type variable)__ と呼ばれるものです。こちらは、プログラマが指定する`'T`のような型変数と同種のものになります。中身は単なる文字列で、自由型変数のように具体的な型とリンクする機能はありません。

未束縛型変数を量化型変数に置き換える操作を __一般化(generalization)__ (または汎化)と言います。未束縛型変数のままだと一回の呼び出しで型が固定化されてしまう(例えば、`id True`の呼び出しの後`id 1`の戻り値型が`Bool`になってしまう)ため、一般化しなくてはならないのです。
このようにして量化型変数を含む一般化された定義が型環境に登録されます。

型変数は最終的に多相関数(クラス)にしか現れませんが、単相関数(クラス)の型推論でも型変数自体は使われていることに注意してください。
これは、ある関数が多相か単相かは実際に型検査を行わなければ判定できないからです。
関数の型検査の結果、全ての型変数が除去できたら単相、できなければ多相となるのです。

## 一般化、型スキーム、具体化

未束縛型変数`?T`を __一般化(generalization)__ する操作を`gen`と表すことにします。このとき、得られる一般化型変数を`|T: Type| T`とします。
型理論では、量化された型、例えば多相関数型`α->α`はその前に`∀α.`を付けて区別します(∀のような記号を(全称)量化子といいます)。
このような表現(e.g. `∀α. α->α`)を型スキームと呼びます。Ergでの型スキームは`|T: Type| T -> T`などと表されます。
型スキームは、通常は第一級の型とはみなされません。そのように型システムを構成すると、型推論がうまく動作しなくなる場合があるためです。

さて、得られた型スキーム(e.g. `'T -> 'T (idの型スキーム)`)を使用箇所(e.g. `id 1`, `id True`)の型推論で使う際は、一般化を解除する必要があります。この逆変換を __具体化(instantiation)__ と呼びます。操作は`inst`と呼ぶことにします。

```python
gen ?T = 'T
inst 'T = ?T (?T ∉ Γ)
```

重要な点として、どちらの操作も、その型変数が出現する場所すべてを置換します。例えば、`'T -> 'T`を具体化すると、`?T -> ?T`が得られます。
具体化の際は置換用のDictが必要ですが、一般化の際は`?T`に`'T`をリンクさせるだけで置換できます。

あとは引数なりの型を与えて目的の型を得ます。この操作を型代入(Type substitution)といい、`subst`と表すことにします。
さらに、その式が呼び出しの場合に戻り値型を得る操作を`subst_call_ret`と表します。第1引数は引数型のリスト、第2引数は代入先の型です。

型代入規則`{?T --> X}`は、`?T`と`X`を同一の型とみなすよう書き換えるという意味です。この操作を __単一化(Unification)__ といいます。`X`は型変数もありえます。
単一化の詳しいアルゴリズムは[別の項](./unification.md)で解説します。単一化操作は`unify`と表すことにします。

```python
unify(?T, Int) == Ok(()) # ?T == (Int)

# Sは型代入規則、Tは適用する型
subst(S: {?T --> X}, T: ?T -> ?T) == X -> X
# 型代入規則は{?T --> X, ?U --> T}
subst_call_ret([X, Y], (?T, ?U) -> ?U) == Y
```

## 半単一化

単一化の亜種として、半単一化(__Semi-unification__)というものがあります。これは、部分型関係を満たすように型変数の制約を更新する操作です。Ergはの型システムは部分型を持つので、半単一化を多用します。
この操作では、型変数は場合によって単一化できたりできなかったりするので、「半」単一化と呼ばれます。

半単一化は、引数の代入時などに行われます。
というのも、実引数の型は、仮引数の型のサブタイプでなければなりません。
引数の型が型変数の場合は、それを満たすように部分型関係を更新する必要があるのです。

```python
# 仮引数の型をTとすると
f(x: T): T = ...

a: U
# U <: Tでなくてはならない、さもなければ型エラー
f(a)
```

半単一化を実現するために、現在の型変数は上界型と下界型を持っています。上界型とは、ある型変数が少なくともその型自身かその汎化型になるような型です。下界型はその逆です。具体的には、何の制約もない型変数は以下のように表されます。

```erg
?T(:> Never, <: Obj)
```

`Never`が下界型で、`Obj`が上界型です。`Never`はあらゆる型のサブタイプであり、`Obj`はあらゆる型のスーパータイプです。なので、この型変数はこの時点ではいかなる型にもなれます。

```erg
?U(:> Nat, <: Int)
```

この型は、`Nat`以上`Int`以下の型です。`Nat`と`Int`はもちろん適合しますし、`Nat or {-1}`といった型でも適合します。

具体化された関数型の型変数は、実引数が代入される際に通常下界型が狭められるように半単一化されます。

```erg
# id: |T| T -> T
id True
# instantiate: id: ?T -> ?T
# True: Bool
# sub_unify(sub: Bool, sup: ?T)
# ?T(:> Never, <: Obj) --> ?T(:> Bool, <: Obj)
```

型検査が終わった後のErg HIRに自由型変数(`?T`など)は存在しません。全て、量化型変数か具体的な型に置換されます。この操作をdereference(deref, 型変数除去)と呼びます。
上のコードでこのまま型検査が終われば、型は最も小さくなるようにderefされます。関数型は引数に関して反変、戻り値に関して共変なので、引数の`?T`は`Obj`に、戻り値の`?T`は`Bool`に置換されます。

```erg
(id: ?T(:> Bool, <: Obj) -> ?T(:> Bool, <: Obj))(True: Bool): ?T(:> Bool, <: Obj)
# ↓
(id: Obj -> Bool)(True: Bool): Bool
```

## 一般化

部分型のことは一旦脇において、型変数の一般化の話題に移ります。

一般化は単純な作業ではありません。複数のスコープが絡むと、型変数の「レベル管理」が必要になります。
レベル管理の必要性をみるために、まずはレベル管理を導入しない型推論では問題が起こることを確認します。
以下の無名関数の型を推論してみます。

```python
x ->
    y = x
    y
```

まず、Ergは以下のように型変数を割り当てます。
yの型も未知ですが、現段階では割り当てないでおきます。

```python
x(: ?T) ->
    y = x
    y
```

まず決定すべきは右辺値xの型です。右辺値は「使用」なので、具体化します。
しかしxの型`?T`は自由変数なのですでに具体化されています。よってそのまま`?T`が右辺値の型になります。

```python
x(: ?T) ->
    y = x (: inst ?T)
    y
```

左辺値yの型として登録する際に、一般化します。が、後で判明するようにこの一般化は不完全であり、結果に誤りが生じます。

```python
x(: ?T) ->
    y(: gen ?T) = x (: ?T)
    y
```

```python
x(: ?T) ->
    y(: 'T) = x
    y
```

yの型は量化型変数`'T`となりました。次の行で、`y`が早速使用されています。具体化します。

```python
x: ?T ->
    y(: 'T) = x
    y(: inst 'T)
```

ここで注意してほしいのが、具体化の際にはすでに存在するどの(自由)型変数とも別の(自由)型変数を生成しなくてはならないという点です(一般化も同様です)。このような型変数をフレッシュ(新鮮)な型変数と呼びます。

```python
x: ?T ->
    y = x
    y(: ?U)
```

そして得られた全体の式の型を見てください。`?T -> ?U`となっています。引数と戻り値の型が別物だという結果が得られました。しかし明らかにこの式は`?T -> ?T`のはずで、推論に問題があるとわかります。
こうなったのは、型変数の「レベル管理」を行っていなかったからです。

そこで、型変数のレベルを以下の表記で導入します。レベルは自然数で表します。

```python
# 通常のType型変数
?T<1>, ?T<2>, ...
# 部分型制約を付けられた型変数
?T<1>(<: U) or ?T(<: U)<1>, ...
```

では、リトライしてみます。

```python
x ->
    y = x
    y
```

まず、以下のようにレベル付き型変数を割り当てます。トップレベルのレベルは1です。スコープが深くなるたび、レベルが増えます。
関数の引数は内側のスコープに属するため、関数自身より1大きいレベルにいます。

```python
# level 1
x (: ?T<2>) ->
    # level 2
    y = x
    y
```

先に右辺値`x`を具体化します。先ほどと同じで、何も変わりません。

```python
x (: ?T<2>) ->
    y = x (: inst ?T<2>)
    y
```

ここからがキモです。左辺値`y`の型に代入する際の一般化です。
さきほどはここで結果がおかしくなっていましたので、一般化のアルゴリズムを変更します。
もし型変数のレベルが現在のスコープのレベル以下なら、一般化しても変化がないようにします。

```python
gen ?T<n> = if n <= current_level, then= ?T<n>, else= 'T
```

```python
x (: ?T<2>) ->
    # current_level = 2
    y (: gen ?T<2>)  = x (: ?T<2>)
    y
```

つまり、左辺値`y`の型は`?T<2>`です。

```python
x (: ?T<2>) ->
    #    ↓ not generalized
    y (: ?T<2>)  = x
    y
```

yの型は未束縛型変数`?T<2>`となりました。次の行で具体化します。が、`y`の型は一般化されていないので、何も起こりません。

```python
x (: ?T<2>) ->
    y (: ?T<2>) = x
    y (: inst ?T<2>)
```

```python
x (: ?T<2>) ->
    y = x
    y (: ?T<2>)
```

無事に、正しい型`?T<2> -> ?T<2>`を得ました。

もう1つの例を見ます。こちらは更に一般的なケースで、関数・演算子適用、前方参照がある場合です。

```python
f x, y = id(x) + y
id x = x

f 10, 1
```

1行ずつ見ていきましょう。

`f`の推論中、後に定義される関数定数`id`が参照されています。
このような場合、`f`の前に`id`の宣言を仮想的に挿入し、自由型変数を割り当てておきます。
このときの型変数のレベルは`current_level`であることに注意してください。これは、他の関数内で一般化されないようにするためです。

```python
id: ?T<1> -> ?U<1>
f x (: ?V<2>), y (: ?W<2>) =
    id(x) (: subst_call_ret([inst ?V<2>], inst ?T<1> -> ?U<1>)) + y
```

型変数同士の単一化では、高いレベルの型変数が低いレベルの型変数に置き換えられます。
レベルが同じ場合はどちらでも構いません。

型変数同士の半単一化では、少し事情が違います。
違うレベルの型変数の場合、相互に型制約をかけてはいけません。

```python
# BAD
f x (: ?V<2>), y (: ?W<2>) =
    # ?V<2>(<: ?T<1>)
    # ?T<1>(:> ?V<2>)
    id(x) (: ?U<1>) + y (: ?W<2>)
```

こうすると、型変数を具体化する場所を決定できなくなります。
Type型変数同士の場合、半単一化ではなく、通常の単一化を行います。
つまり、レベルの低い方に単一化させます。

```python
# OK
f x (: ?V<2>), y (: ?W<2>) =
    # ?V<2> --> ?T<1>
    id(x) (: ?U<1>) + y (: ?W<2>)
```

```python
f x (: ?T<1>), y (: ?W<2>) =
    (id(x) + x): subst_call_ret([inst ?U<1>, inst ?W<2>], inst |'L <: Add('R)| ('L, 'R) -> 'L.Output)
```

```python
f x (: ?T<1>), y (: ?W<2>) =
    (id(x) + x): subst_call_ret([inst ?U<1>, inst ?W<2>], (?L(<: Add(?R<2>))<2>, ?R<2>) -> ?L<2>.Output)
```

```python
id: ?T<1> -> ?U<1>
f x (: ?T<1>), y (: ?W<2>) =
    # ?U<1>(<: Add(?W<2>)) # ?Lの制約を引き継ぐ
    # ?L<2> --> ?U<1>
    # ?R<2> --> ?W<2> (?R(:> ?W), ?W(<: ?R)とはしない)
    (id(x) + x) (: ?U<1>.Output)
```

```python
# current_level = 1
f(x, y) (: gen ?T<1>, gen ?W<2> -> gen ?U<1>.Output) =
    id(x) + x
```

```python
id: ?T<1> -> ?U<1>
f(x, y) (: |'W: Type| (?T<1>, 'W) -> gen ?U<1>(<: Add(?W<2>)).Output) =
    id(x) + x
```

```python
f(x, y) (: |'W: Type| (?T<1>, 'W) -> ?U<1>(<: Add(?W<2>)).Output) =
    id(x) + x
```

定義の際には一般化できるようにレベルを上げます。

```python
# ?T<1 -> 2>
# ?U<1 -> 2>
id x (: ?T<2>) -> ?U<2> = x (: inst ?T<2>)
```

戻り値型が既に割り当てられている場合は、得られた型と単一化します(`?U<2> --> ?T<2>`)。

```python
# ?U<2> --> ?T<2>
f(x, y) (: |'W: Type| (?T<2>, 'W) -> ?T<2>(<: Add(?W<2>)).Output) =
    id(x) + x
# current_level = 1
id(x) (: gen ?T<2> -> gen ?T<2>) = x (: ?T<2>)
```

型変数が単なるType型変数に具体化されてしまった場合は、
それに依存する型変数も同じくType型変数になります。
一般化された型変数は各関数で独立です。

```python
f(x, y) (: |'W: Type, 'T <: Add('W)| ('T, 'W) -> 'T.Output) =
    id(x) + x
id(x) (: |'T: Type| 'T -> gen 'T) = x
```

```python
f x, y (: |'W: Type, 'T <: Add('W)| ('T, 'W) -> 'T.Output) =
    id(x) + y
id(x) (: 'T -> 'T) = x

f(10, 1) (: subst_call_ret([inst {10}, inst {1}], inst |'W: Type, 'T <: Add('W)| ('T, 'W) -> 'T.Output)
```

```python
f(10, 1) (: subst_call_ret([inst {10}, inst {1}], (?T<1>(<: Add(?W<1>)), ?W<1>) -> ?T<1>.Output))
```

型変数は、実装のある最小の型まで上限が拡大されます。

```python
# ?T(:> {10} <: Add(?W<1>))<1>
# ?W(:> {1})<1>
# ?W(:> {1})<1> <: ?T<1> (:> {10}, <: Add(?W(:> {1})<1>))
# 直列化すると
# {1} <: ?W<1> or {10} <: ?T<1> <: Add({1}) <: Add(?W<1>)
# Add(?W)(:> ?V)の最小実装トレイトはAdd(Nat) == Natで、Addは第一引数に関して共変なので
# {10} <: ?W<1> or {1} <: ?T<1> <: Add(?W<1>) <: Add(Nat) == Nat
# ?T(:> ?W(:> {10}) or {1}, <: Nat).Output == Nat # 候補が一つしかない場合は評価を確定する
f(10, 1) (: (?W(:> {10}, <: Nat), ?W(:> {1})) -> Nat)
# これでプログラムは終わりなので、型変数を除去する
f(10, 1) (: ({10}, {1}) -> Nat)
```

結果として、プログラム全体の型はこうなります。

```python
f|W: Type, T <: Add(W)|(x: T, y: W): T.Output = id(x) + y
id|T: Type|(x: T): T = x

f(10, 1): Nat
```

元の明示的に型付けられていないプログラムも再掲しておきます。

```python
f x, y = id(x) + y
id x = x

f(10, 1)
```
