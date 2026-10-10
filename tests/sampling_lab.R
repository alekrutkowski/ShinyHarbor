# Run with Rscript --vanilla tests/sampling_lab.R (requires only Shiny).
example <- new.env(parent = globalenv())
sys.source("examples/hello/app.R", envir = example)
set.seed(87)
before <- .Random.seed
e <- example$simulate_experiment("normal", 30L, 2, 3, .95, 5000L, 2026L)
stopifnot(identical(before, .Random.seed), length(e$sample) == 30,
          nrow(e$intervals) == 5000, e$coverage > .93, e$coverage < .97,
          isTRUE(all.equal(e$interval, as.numeric(t.test(e$sample)$conf.int))),
          isTRUE(all.equal(e$se, sd(e$sample) / sqrt(30))),
          isTRUE(all.equal(e, example$simulate_experiment("normal", 30L, 2, 3, .95, 5000L, 2026L))))
for (shape in c("normal", "skewed", "heavy")) {
  sample <- example$simulate_experiment(shape, 5L, 0, 1, .90, 500L, 73L)
  stopifnot(all(is.finite(sample$intervals$lower)), all(sample$intervals$lower <= sample$intervals$upper),
            sample$coverage == mean(sample$intervals$contains_true_mean))
  area <- integrate(function(x) example$population_density(x, sample), -Inf, Inf)$value
  stopifnot(abs(area - 1) < .001)
}
shiny::testServer(example$server, {
  session$setInputs(shape = "normal", n = 30, mu = 0, sigma = 1, level = ".95", repetitions = "500", seed = 2026)
  first <- experiment()
  session$setInputs(n = 100)
  stopifnot(!identical(first, experiment()))
  stopifnot(experiment()$n == 100, length(experiment()$sample) == 100)
  session$setInputs(mu = 3, sigma = 2, level = '.99', shape = 'skewed', repetitions = '2000')
  stopifnot(experiment()$mu == 3, experiment()$sigma == 2, experiment()$level == .99,
            experiment()$shape == 'skewed', nrow(experiment()$intervals) == 2000)
})
cat("Sampling Lab: interval formula, densities, coverage, seed isolation, reproducibility and reactive controls passed.\n")
