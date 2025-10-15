# Zabbix → Telegram бот (Rust)

Rust-реализация бота для получения алертов от Zabbix в Telegram. Бот использует токен от @BotFather и список разрешенных пользователей (allowed_users.txt). Для отправки алертов используется встроенный в Zabbix Media type "Telegram" — отдельный HTTP вебхук на стороне бота не нужен.

Подробнее про настройку Zabbix см. в файле `docs/ZABBIX_SETUP.md`.

## Возможности
- Команды:
  - `/start` — проверка доступа (пользователь должен быть в allowed_users.txt)
  - `/help` — список команд
  - `/id` — вернет ваш Telegram ID
- Читает список разрешенных пользователей из файла `allowed_users.txt` (по умолчанию: `/bot/allowed_users.txt`).
- Работает в Docker, минимум зависимостей.

## Быстрый старт

1) Создайте бота через @BotFather и получите токен.

2) Подготовьте директории на хосте:

```bash
sudo mkdir -p /docker_sys/zabbix_tg
sudo chown $USER:$USER /docker_sys/zabbix_tg
```

3) Скопируйте файлы проекта на сервер (или просто клонируйте репозиторий).

4) Создайте `.env` на основе примера и вставьте токен:

```bash
cp .env.example .env

```

5) Подготовьте `allowed_users.txt` (один ID на строку, можно использовать комментарии с `#`):

```bash
echo "# список разрешенных пользователей" > /docker_sys/zabbix_tg/allowed_users.txt
echo "1349552926" >> /docker_sys/zabbix_tg/allowed_users.txt
```

6) Соберите и запустите контейнер:

```bash
docker compose up -d --build
```

Проверьте логи:

```bash
docker logs -f zabbix-tg
```

7) Узнайте свой Telegram ID:
- Напишите этому боту команду `/id` (бот ответит вашим ID), либо
- Используйте @getmyid_bot и добавьте ваш ID в `allowed_users.txt`.

## Настройка Zabbix (через встроенный media type "Telegram")

1) Zabbix → "Administration" → "Media types" → "Telegram" → "Test".
   - В поле "To" укажите ID чата (ваш ID или ID группы/канала, если бот добавлен в них и имеет право писать).
   - В поле "Token" укажите токен вашего бота (тот же, что в `.env`).

2) Zabbix → "Administration" → "Users" → выберите пользователя (например, Admin).
   - В разделе "Media" добавьте/измените запись, указав ID чата в "Send to" (и токен бота, если требуется шаблоном).

3) Zabbix → "Configuration" → "Actions" → "Create action".
   - Subject: `{HOST.NAME} | Problem: {EVENT.NAME}`
   - Message:
     ```
     Problem started at {EVENT.TIME} on {EVENT.DATE}
     Problem name: {EVENT.NAME}
     Host: {HOST.NAME}
     Severity: {TRIGGER.SEVERITY}
     Original problem ID: #{EVENT.ID}
     {TRIGGER.URL}
     ```

4) Проверка: вызовите триггер (например, измените IP на неверный) и убедитесь, что сообщение пришло в Telegram.

Примечание: Бот на стороне Telegram не обязан быть "онлайн" для доставки сообщений через Telegram API — Zabbix отправляет сообщения напрямую по токену. Наш бот полезен для проверки права доступа и команд сервиса.

## Переменные окружения
- `TELEGRAM_BOT_TOKEN` — токен бота от @BotFather (обязательно).
- `ALLOWED_USERS_PATH` — путь к файлу со списком разрешенных пользователей (по умолчанию `/bot/allowed_users.txt`).

## Структура томов в docker-compose
- Том `/docker_sys/zabbix_tg` на хосте монтируется как `/bot` в контейнере.
- Файл `/docker_sys/zabbix_tg/allowed_users.txt` читается ботом при старте.

## Разработка локально

```bash
# Сборка
cargo build --release

# Запуск с токеном в окружении
TELEGRAM_BOT_TOKEN=... ALLOWED_USERS_PATH=./allowed_users.txt \
  ./target/release/zabbixbot
```

## Безопасность
- Никогда не коммитьте реальный токен в репозиторий. Используйте `.env` и переменные окружения.
- Для продакшна в Zabbix лучше создать отдельного пользователя, чем использовать Admin.
