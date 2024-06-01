# Harmonia jako projekt badawczo-rozwojowy

Katalog łączy kod projektu z dokumentacją projektu badawczo rozwojowego.

## Budowanie dokumentacji

```console
asciidoctor -o architecture.html architecture.adoc
```

## Planowana architektura, a implementacja Harmonii

Oryginalnie zidentyfikowano następujące komponenty systemu:

1. interfejs graficzny w postaci serwera HTTP
1. wątek audio wykonujący fragment z użyciem mechanizmu synchronizacji
1. mechanizm synchronizacji: rozbudowa biblioteki Ableton/Link
1. serwer mDNS dający dodatkowe współdzielenie informacji w sieci

Zrealizowano 3 pierwsze komponenty systemu, będące równocześnie głównymi komponentami systemu.

### Interfejs graficzny w postaci serwera HTTP

Serwer HTTP został zrealizowany przy pomocy biblioteki [Axum](https://github.com/tokio-rs/axum), z werstwą interaktywną zdefiniowną przy pomocy biblioteki [HTMX](https://htmx.org/) oraz [dedykowane rozwiązania w języku JavaScript](../public/index.js) i [CSS](../public/index.css).

By zapewnić minimalne zużycie zasobów interakcje definiowane po stronie klienta (w ramach przeglądarki) są minimalne, a HTML jest przesyłany w pełnej formie przy pomocy statynych szabolnów z wykorzystaniem biblioteki [maud](https://maud.lambda.xyz/).

Stan serwera jest raportowany na bieżąco przez serwer do klienta przy wykorzystaniu WebSockets.

Zrealizowano wszystkie zakładane elementy interfejsu graficznego.

### Wątek audio wykonujący fragment z użyciem mechanizmu synchronizacji

Wykorzystując środowisko asynchroniczne [tokio](https://tokio.rs/) stworzono wątek audio, komunikujący się asynchronicznie z wątkiem serwera i wątkiem synchronizacyjnym przy wykorzystaniu kanałów.

Wątek audio zarządza wykonywaniem "bloków" (abstrakcja dowolnego typu danych, który Harmonia może odtworzyć synchronicznie), zlecając synchroniczny start na polecenie użytkownika jak i realizując wykonanie samego utworu.

W przypadku plików MIDI dodatkowo zapamiętuje wydawane polecenia by w przypadku przerwania wykonania móc zakońćzyć odtwarzanie zleconych wcześniej nut.

Zrealizowano wszystkie zakładane elementy wątku audio (moduł `audio_engine` wewnątrz implementacji), z dodatkowym rozszerzeniem umożliwiającym tworzenie prostych środowisk programowania muzycznego w oparciu o komunikację międzyprocesową z wykorzystaniem współdzielonej pamięci.

### Mechanizm synchronizacji: rozbudowana biblioteka Ableton Link

Podstawą mechanizmu synchronizacji jest biblioteka Ableton Link, której implementacja pozwoliła na nakreślenie teoretycznej podbudowy projektu.
Kilka iteracji metod rozbudowy biblioteki doprowadziło do aktualnie wdrożonego mechanizmu, polegającego na złamaniu enkapsulacji i współdzieleniu informacji wewnętrznych biblioteki z Harmonią (commit [0ea5c57](https://github.com/RobertBendun/link/commit/0ea5c5725447829af28f282f91af8a94e28fbabe) w repozytorium [link](https://github.com/RobertBendun/link)).

Złamanie abstrakcji pozwoliło na [implementację mechanizmu partycypacyjnego startu](https://github.com/RobertBendun/harmonia/blob/main/src/linky_groups.rs) opartego na podobnej mechanice do biblioteki Ableton Link, z różnicą w wykorzystywanym stanie i rozwiązywaniu jego różnic.

### serwer mDNS udostępniający podstawowe informacje o instancji

Dla potrzeb orkiestry laptopowej mechanizm współpracy oparty o przekazywanie adresów IP dostępnych w interfejsie został uznany jako wystarczający.
Implementacja protokołu mDNS została tymczasowo zawieszona na rzecz integracji z środowiskami muzyki algorytmicznej.
