# Egress

```yaml
egress:
  mode: direct
  # single_proxy — один прокси
  # pools — enterprise
```

`tunnel` — через `wss` агента.

## SOCKS и живой egress (0.1.1)

Обе ветки умеют `socks5h://` (`httpx[socks]`, reqwest `socks`; без них single_proxy на SOCKS ронял Python-гейт на старте). Проверено живьём: чаты через `single_proxy` до OpenRouter — 200 в обеих ветках.

Публичные прокси дохнут за часы (проверено) и светят трафик чужим exit-нодам — для продакшена нужен свой прокси. Метод отбора: оракул `/api/v1/auth/key` с мусорным ключом (401 = exit IP допущен, 403 = заблокирован; настоящий ключ при этом не покидает контур).
