# Parametri di configurazione del Puzzle Solver

Tutti i parametri si passano da riga di comando **senza ricompilare**.  
Formato generale:

```
puzzle --parametro VALORE [--altro-parametro VALORE ...]
```

---

## Come funziona il punteggio

Il programma confronta coppie di lati (uno con tab, uno con hole) e calcola un **punteggio di compatibilità** da 0 a 100.  
Il punteggio finale è una media pesata di sette componenti:

```
score = (eu * W_eu + pe * W_pe + de * W_de + po * W_po + ar * W_ar + cm * W_cm + cx * W_cx)
      / (W_eu + W_pe + W_de + W_po + W_ar + W_cm + W_cx)
```

Solo i lati con `score >= threshold` compaiono nel file `output.json`.

---

## `--threshold`

**Cosa fa:** soglia minima del punteggio finale (0–100) per includere una coppia di lati nell'output. Coppie con punteggio più basso vengono scartate silenziosamente.

**Default:** `80.0`

**Come cambiarlo:**
```
puzzle --threshold 85
```

**Esempio:** con `--threshold 90` vengono tenute solo le coppie molto simili; abbassarlo a `70` include più candidati ma con più falsi positivi.

---

## `--weight-euclidean`

**Cosa fa:** peso della **distanza euclidea** dell'apice della concavità dai due angoli del lato (corner_a e corner_b). Misura quanto le distanze geometriche dirette tab↔hole si assomigliano.

**Default:** `0.10`

**Come cambiarlo:**
```
puzzle --weight-euclidean 0.30
```

**Esempio:** alzarlo a `0.30` rende il confronto più sensibile alla posizione spaziale dell'apice; utile se i pezzi sono fotografati sempre alla stessa scala.

---

## `--weight-perimeter`

**Cosa fa:** peso della **distanza perimetrale** dell'apice lungo il bordo del pezzo (quanti pixel di contorno separano l'apice da ciascun angolo). Complementare alla distanza euclidea: cattura la forma del bordo tra apice e angolo.

**Default:** `0.10`

**Come cambiarlo:**
```
puzzle --weight-perimeter 0.20
```

**Esempio:** alzarlo favorisce lati con percorso di bordo simile, indipendentemente da piccole variazioni angolari.

---

## `--weight-depth`

**Cosa fa:** peso della **profondità della concavità** — la distanza perpendicolare massima tra l'apice e la retta che unisce i due angoli del lato. Tab e hole abbinati devono avere profondità simile.

**Default:** `0.10`

**Come cambiarlo:**
```
puzzle --weight-depth 0.25
```

**Esempio:** alzarlo è utile quando i pezzi hanno tab/hole di profondità molto diversa tra loro (puzzle con forme irregolari), perché penalizza di più le coppie con profondità incompatibili.

---

## `--weight-position`

**Cosa fa:** peso del **rapporto di posizione dell'apice** lungo la baseline — dove si trova l'apice tra i due angoli, espresso come valore 0.0 (vicino a corner_a) … 1.0 (vicino a corner_b). Viene confrontato anche nella versione speculare (1 − ratio) per gestire eventuali orientamenti ribaltati.

**Default:** `0.10`

**Come cambiarlo:**
```
puzzle --weight-position 0.20
```

**Esempio:** alzarlo è utile quando i tab sono molto asimmetrici (non centrati) e si vuole che due lati si abbinino solo se l'apice cade nella stessa zona relativa.

---

## `--weight-area`

**Cosa fa:** peso della **area della concavità** — l'area del poligono delimitato dal contorno del lato e dalla retta baseline tra i due angoli (formula di Shoelace). Tab e hole che si incastrano devono avere aree simili.

**Default:** `0.50`

**Come cambiarlo:**
```
puzzle --weight-area 0.30
```

**Esempio:** di default ha il peso maggiore perché l'area integra in un solo numero la profondità e la larghezza della concavità. Abbassarlo a `0.20` lo mette sullo stesso piano degli altri indicatori; alzarlo a `0.80` rende l'area quasi l'unico criterio decisivo.

---

## `--weight-contour-mean`

**Cosa fa:** peso del **Metodo 1** di confronto del profilo di contorno — la **media** di `|d_A[t] + d_B[t]|` su 100 punti equidistanti lungo la baseline. Il profilo `d[t]` è la distanza perpendicolare firmata (positiva = verso l'interno del pezzo) normalizzata per la lunghezza del lato. Per una coppia perfetta tab+hole la somma dovrebbe essere zero in ogni punto.

**Default:** `0.10`

**Come cambiarlo:**
```
puzzle --weight-contour-mean 0.30
```

**Esempio:** alzarlo a `0.30` rende il punteggio molto sensibile alla forma complessiva del bordo; se due lati hanno profili complementari ma leggermente traslati, la media penalizza meno del massimo (vedi `--weight-contour-max`).

---

## `--weight-contour-max`

**Cosa fa:** peso del **Metodo 2** di confronto del profilo di contorno — il **massimo** di `|d_A[t] + d_B[t]|` (distanza di Hausdorff monodimensionale). Rispetto alla media, è più severo: basta un singolo punto mal allineato per abbassare il punteggio.

**Default:** `0.10`

**Come cambiarlo:**
```
puzzle --weight-contour-max 0.30
```

**Esempio:** alzarlo a `0.30` scarta le coppie in cui anche solo una piccola zona del bordo non combacia; utile per puzzle con bordi molto precisi. Abbassarlo a `0.0` disabilita di fatto il Metodo 2 (resta solo il Metodo 1).

---

## `--contour-threshold`

**Cosa fa:** soglia relativa per il **gate** del confronto contorno. Valore normalizzato rispetto alla lunghezza del lato (baseline): `0.15` significa che la differenza media/massima tra i profili non deve superare il 15% della lunghezza del lato. Se **entrambi** i metodi superano questa soglia, la coppia viene scartata prima ancora di calcolare il punteggio finale (ritorna 0).

**Default:** `0.15`

**Come cambiarlo:**
```
puzzle --contour-threshold 0.10
```

**Esempio:**  
- `--contour-threshold 0.05` → gate molto stretto, scarta qualsiasi coppia con profili non quasi-perfetti; riduce i falsi positivi ma rischia di eliminare match reali se le foto sono leggermente distorte.  
- `--contour-threshold 0.30` → gate largo, quasi nessuna coppia viene scartata dal gate; il confronto contorno contribuisce al punteggio ma non esclude nessuno.  
- `--contour-threshold 0.0` → disabilita completamente il gate (nessuna coppia viene mai scartata per il profilo contorno).

---

## Esempio completo

Eseguire con soglia alta, area molto importante e gate contorno stretto:

```
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
  --contour-threshold 0.08
```

In questo caso l'area della concavità vale il 50% del punteggio, e solo le coppie il cui profilo di bordo differisce meno dell'8% della lunghezza del lato vengono considerate.
