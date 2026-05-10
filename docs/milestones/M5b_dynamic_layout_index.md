# Milestone 5b — динамический layout index

> Дата: 2026-05-10. Маленький correctness-fix поверх M5.

## Проблема

В M5 первоначально было:
```rust
let target_index: u32 = match t.layout.as_str() {
    "us" => 1,
    "ru" => 0,
    _ => return Ok(()),
};
```

Это hardcode предполагает что в `kxkbrc` `LayoutList=us,ru` (us=index 0, ru=index 1).
Если у юзера в System Settings → Keyboard layouts указаны **в обратном
порядке** (`LayoutList=ru,us`) или есть **третья раскладка** (немецкий /
украинский / etc.) — наш `setLayout(1)` пошлёт юзера в случайный layout.

## Fix

В `KwinLayout::new()` теперь дёргаем `getLayoutsList()` через DBus
(`org.kde.KeyboardLayouts.getLayoutsList -> a(sss)`). Парсим в
`HashMap<String, u32>` (xkb-имя → реальный индекс). Сохраняем в struct.

В `do_flip` теперь:
```rust
let target_name = match t.layout.as_str() {
    "us" => "ru",
    "ru" => "us",
    _ => return Ok(()),
};
let Some(target_index) = kwin.index_of(target_name) else {
    warn!("target раскладка отсутствует в KWin LayoutList");
    return Ok(());
};
```

## Дополнительно

В `KwinLayout::new()` теперь логируем все обнаруженные layouts:
```
INFO ... index=0 short=us display="English (US)" long=... KWin layout
INFO ... index=1 short=ru display="Russian"      long=... KWin layout
```

Это видно сразу при старте matea — agent на следующей сессии увидит
конфигурацию системы пользователя.

Если у пользователя нет `ru` в LayoutList, matea **не упадёт** — просто
пропустит FLIP с warning. После добавления раскладки в System Settings
matea подхватит её на следующем перезапуске.

## Не сделано (отложено в M5c)

- **Wait for `layoutChanged` signal** вместо 50мс sleep'а после `set_layout()`.
  Технически правильнее, ниже latency на быстрых системах. Реализация:
  signal subscription через zbus proxy `receive_layout_changed()`.
- **`EVIOCGRAB` на event15** во время rewrite — закроет race window
  когда юзер успевает напечатать новый символ посреди backspace+replay.
- **Auto-reload `name_to_index` map** на signal `layoutListChanged` — если
  юзер в System Settings → Keyboard поменяет layouts, matea сейчас не
  узнает (нужно перезапустить).
