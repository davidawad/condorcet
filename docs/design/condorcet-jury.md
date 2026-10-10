# Condorcet jury theorem as a composer

Status: design note.

## Idea

Treat each independent exhibit (or source) bearing on a binary claim as a
juror. Juror i votes for the claim or against it and is correct with
probability p_i. The composer returns the probability that the majority vote
is correct.

## Result

Condorcet (1785) showed that for n odd jurors, each independently correct
with the same probability p, the probability the majority is correct is

    P_n(p) = sum_{k=(n+1)/2}^{n} C(n,k) p^k (1-p)^(n-k)

- p > 1/2: P_n increases with n and tends to 1.
- p = 1/2: P_n = 1/2 for every n.
- p < 1/2: P_n decreases with n and tends to 0.

Worked values at p = 0.6: P_3 = 0.648, P_5 = 0.68256.

Heterogeneous competence (p_i differs): P is the probability that a
Poisson-binomial count of correct votes exceeds n/2. A composer computes it
by dynamic programming over jurors in O(n^2), no sampling.

## Failure under correlated jurors

The theorem needs the votes to be independent given the truth. Exhibits that
share a source, method or cause are not independent
(they double-count the same evidence).
Counting five copies of one wire story as five jurors reports P_5 = 0.68 when
the real answer is P_1 = 0.6.

Illustrative common-cause model: with probability rho all jurors copy one
shared draw (correct with probability p), otherwise they vote independently.
Then

    P = rho * p + (1 - rho) * P_n(p)

and as n grows P tends to rho * p + (1 - rho), not 1. With p = 0.6 and
rho = 0.3 the ceiling is 0.3 * 0.6 + 0.7 = 0.88: more jurors never beat it.
With strong correlation and p just above 1/2 the majority can be no better
than a single juror.

Composer rules:

1. Collapse each independence cluster in the bundle (`clusters`) to one juror,
   or weight its members by (1 - rho), matching the kd cluster rule
   `L_c = l_1 + (1 - rho_c) * sum(l_2..l_n)`.
2. Require p_i > 0.5 per juror; a juror with p_i <= 0.5 carries no positive
   information and is dropped and reported in the run evidence.
3. Always emit the ceiling rho * p + (1 - rho) next to the score, so the
   consumer sees what adding jurors cannot fix.
4. p_i comes from the adapter (rules of evidence), never from the engine.

## Sources

- Condorcet, J.-A.-N. de Caritat, marquis de (1785). Essai sur l'application
  de l'analyse a la probabilite des decisions rendues a la pluralite des
  voix. Paris: Imprimerie Royale. BnF Gallica record:
  https://gallica.bnf.fr/ark:/12148/bpt6k417181 . Verified: the Gallica-sourced
  PDF at https://www.hist-math.fr/textes/Condorcet1785_ProbabiliteDecisions.pdf
  carries this exact title and "Source gallica.bnf.fr" (fetched 2026-10-10).
  Direct Gallica access from the CLI returned a Cloudflare 403, so the
  Gallica URL itself comes from search results and was not fetched.
- The formula above is the standard modern statement of the theorem; it was
  not located page by page in the Essai.
- Correlated-juror results (Ladha 1992, Dietrich and Spiekermann 2013) are
  UNVERIFIED here: named from memory, not fetched. The common-cause model
  above is an illustration, not taken from either paper.
