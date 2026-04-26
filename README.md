# Puzzle Solver — Documentazione

Analizza immagini di pezzi di puzzle, estrae descrittori geometrici per ogni lato e calcola quali lati di quali pezzi si collegano. I risultati vengono salvati in `output/output.json`.

---

## Build e avvio

```bash
cargo build --release

puzzle --input ./input --output ./output --threshold 80
```

---

## File prodotti

| File | Descrizione |
|------|-------------|
| `output/output.json` | Tutti i match sopra soglia; ricalcolato ad ogni avvio |
| `output/<id>.json` | Descrittore geometrico del singolo pezzo (cache: riusato se presente) |
| `output/<id>.jpg` | Immagine di debug con angoli, lati e apici evidenziati |
| `output/user.json` | Coppie confermate manualmente dall'utente; persiste tra le sessioni |
| `output/puzzle.log` | Log completo dell'ultima esecuzione |

---

## Loop interattivo

Al termine dell'analisi il programma entra in modalità interattiva (tasto `Esc` per uscire).

### Ispezionare un pezzo

```
> 1
```

Mostra tutti e 4 i lati del pezzo 1 con tipo (`Tab` / `Hole` / `Linear`) e i numeri dei pezzi candidati.

**Leggenda dei numeri candidati:**

| Stile | Significato |
|-------|-------------|
| normale | candidato generico |
| **grassetto** | coppia confermata dall'utente (`user.json`) |
| <u>sottolineato</u> | match mutuale — entrambi i lati si riconoscono a vicenda |
| **<u>grassetto + sottolineato</u>** | confermato dall'utente e mutuale |

### Confermare una coppia

```
> 1 2
```

Collega il pezzo 1 al pezzo 2 con la seguente logica:

- **Pezzo 2 compare in un solo lato di pezzo 1** → le alternative per quel lato vengono eliminate; il lato resta con il solo pezzo 2 (e viceversa per pezzo 2 rispetto a pezzo 1).
- **Pezzo 2 compare in più lati di pezzo 1** → nessuna eliminazione; il numero 2 viene evidenziato in grassetto.

La coppia viene salvata in `user.json` e viene rielaborata automaticamente al prossimo avvio del programma.

---

## Come funziona il punteggio

Il programma confronta coppie di lati (uno con tab, uno con hole) e calcola un **punteggio di compatibilità** da 0 a 100.  
Il punteggio finale è una media pesata di **otto componenti**:

```
score = (eu·W_eu + pe·W_pe + de·W_de + po·W_po + ar·W_ar
       + cm·W_cm + cx·W_cx + bl·W_bl)
      / (W_eu + W_pe + W_de + W_po + W_ar + W_cm + W_cx + W_bl)
```

Solo i lati con `score >= threshold` compaiono in `output.json`.

**Gate contorno:** se entrambi i metodi di confronto contorno (mean e max) superano `--contour-threshold`, la coppia viene scartata direttamente (score = 0) senza calcolare gli altri componenti.

---

## Parametri

### `--input`
Directory delle immagini dei pezzi.  
**Default:** `input`

### `--output`
Directory per immagini di debug, descrittori JSON e file di output.  
**Default:** `output`

---

### `--threshold`

Soglia minima del punteggio finale (0–100) per includere una coppia nell'output. Coppie con punteggio più basso vengono scartate.

**Default:** `80.0`

```
puzzle --threshold 85
```

---

### `--weight-euclidean`

Peso della **distanza euclidea** dell'apice dai due angoli del lato. Misura quanto le distanze geometriche tab↔hole si assomigliano. Il confronto è sempre incrociato (corner_a di A ↔ corner_b di B) perché i lati adiacenti percorrono il bordo in direzioni opposte.

**Default:** `0.10`

---

### `--weight-perimeter`

Peso della **distanza perimetrale** dell'apice lungo il bordo del pezzo. Cattura la forma del bordo tra apice e angolo, complementare alla distanza euclidea. Anche qui il confronto è incrociato (corner_a di A ↔ corner_b di B).

**Default:** `0.10`

---

### `--weight-depth`

Peso della **profondità della concavità** — la distanza perpendicolare massima tra l'apice e la retta che unisce i due angoli. Tab e hole abbinati devono avere profondità simile.

**Default:** `0.10`

---

### `--weight-position`

Peso del **rapporto di posizione dell'apice** lungo la baseline (0.0 = vicino a corner_a, 1.0 = vicino a corner_b). La formula corretta è `1 − |r_A + r_B − 1|`: poiché i lati adiacenti sono percorsi in direzioni opposte, la posizione `r_A` su un lato corrisponde alla posizione `1 − r_B` sul lato opposto.

**Default:** `0.10`

---

### `--weight-area`

Peso dell'**area della concavità** — l'area del poligono delimitato dal contorno del lato e dalla baseline (formula di Shoelace). Integra in un solo numero profondità e larghezza della concavità.

**Default:** `0.50`

---

### `--weight-contour-mean`

Peso del **Metodo 1** di confronto del profilo di contorno: media di `|d_A[t] + d_B[t]|` su 100 punti equidistanti lungo la baseline. Il profilo `d[t]` è la distanza perpendicolare firmata normalizzata per la lunghezza del lato. Per una coppia perfetta tab+hole la somma dovrebbe essere zero in ogni punto.

**Default:** `0.10`

---

### `--weight-contour-max`

Peso del **Metodo 2** di confronto del profilo di contorno: massimo di `|d_A[t] + d_B[t]|` (distanza di Hausdorff monodimensionale). Più severo della media: basta un singolo punto mal allineato per abbassare il punteggio.

**Default:** `0.10`

---

### `--weight-baseline`

Peso della **lunghezza della baseline** — la distanza retta tra i due angoli che delimitano il lato (corner_a → corner_b). Due lati che si incastrano fisicamente devono avere la stessa lunghezza; questo componente penalizza coppie di lati di dimensione molto diversa.

**Default:** `0.10`

```
puzzle --weight-baseline 0.30
```

---

### `--contour-threshold`

Soglia relativa per il **gate** del confronto contorno, normalizzata rispetto alla lunghezza del lato. Se **entrambi** Metodo 1 e Metodo 2 superano questa soglia, la coppia viene scartata prima di calcolare il punteggio finale.

**Default:** `0.15`

```
puzzle --contour-threshold 0.10
```

| Valore | Effetto |
|--------|---------|
| `0.05` | Gate stretto: scarta quasi tutto ciò che non è quasi-perfetto |
| `0.15` | Bilanciato (default) |
| `0.30` | Gate largo: quasi nessuna coppia viene scartata dal gate |
| `0.0`  | Gate disabilitato |

---

### `--threads`

Numero di thread per l'analisi parallela delle immagini. `0` usa tutti i core disponibili.

**Default:** `0`

```
puzzle --threads 4
```

### `--log-level`

Livello di dettaglio del log: `error` | `warn` | `info` | `debug` | `trace`.

**Default:** `info`

```
puzzle --log-level debug
```

---

## Esempio completo

```bash
puzzle \
  --input ./foto \
  --output ./risultati \
  --threshold 85 \
  --weight-euclidean 0.05 \
  --weight-perimeter 0.05 \
  --weight-depth 0.10 \
  --weight-position 0.10 \
  --weight-area 0.50 \
  --weight-contour-mean 0.10 \
  --weight-contour-max 0.10 \
  --weight-baseline 0.10 \
  --contour-threshold 0.08 \
  --threads 0 \
  --log-level info
```
