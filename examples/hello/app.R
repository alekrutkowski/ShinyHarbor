# Shiny provides interaction; all statistics, graphics and exports use base R.
library(shiny)

with_seed <- function(seed, code) {
  had_seed <- exists(".Random.seed", envir = .GlobalEnv, inherits = FALSE)
  if (had_seed) old_seed <- get(".Random.seed", envir = .GlobalEnv)
  on.exit(if (had_seed) assign(".Random.seed", old_seed, envir = .GlobalEnv)
          else if (exists(".Random.seed", envir = .GlobalEnv, inherits = FALSE))
            rm(".Random.seed", envir = .GlobalEnv))
  set.seed(seed)
  force(code)
}

simulate_experiment <- function(shape, n, mu, sigma, level, repetitions, seed) {
  stopifnot(shape %in% c("normal", "skewed", "heavy"), n >= 5, n <= 200,
            is.finite(mu), sigma > 0, sigma <= 20, level > 0, level < 1,
            repetitions >= 100, repetitions <= 5000, is.finite(seed))
  with_seed(seed, {
    draw <- function(size) mu + sigma * switch(shape, normal = rnorm(size),
      skewed = rexp(size) - 1, heavy = rt(size, df = 3) / sqrt(3))
    sample <- draw(n)
    samples <- matrix(draw(n * repetitions), nrow = n)
    means <- colMeans(samples)
    ses <- sqrt(colSums(sweep(samples, 2, means)^2) / (n - 1) / n)
    critical <- qt((1 + level) / 2, df = n - 1)
    intervals <- data.frame(repetition = seq_len(repetitions), mean = means,
      lower = means - critical * ses, upper = means + critical * ses)
    intervals$contains_true_mean <- intervals$lower <= mu & intervals$upper >= mu
    se <- sd(sample) / sqrt(n)
    coverage <- mean(intervals$contains_true_mean)
    list(shape = shape, n = n, mu = mu, sigma = sigma, level = level,
      repetitions = repetitions, seed = seed, sample = sample, mean = mean(sample),
      se = se, interval = mean(sample) + c(-1, 1) * critical * se,
      intervals = intervals, coverage = coverage,
      coverage_mcse = sqrt(coverage * (1 - coverage) / repetitions))
  })
}
shape_name <- function(x) switch(x, normal = "Bell-shaped", skewed = "Right-skewed", heavy = "Heavy-tailed")
population_density <- function(x, e) {
  z <- (x - e$mu) / e$sigma
  switch(e$shape, normal = dnorm(z), skewed = dexp(z + 1),
    heavy = dt(z * sqrt(3), df = 3) * sqrt(3)) / e$sigma
}
plot_style <- function() par(mar = c(4.5, 4.5, 2, 1), las = 1, bty = "n",
  fg = "#667783", col.axis = "#526575", col.lab = "#263e50", family = "sans")
plot_sample <- function(e) {
  plot_style()
  h <- hist(e$sample, breaks = "FD", plot = FALSE)
  x <- seq(min(h$breaks), max(h$breaks), length.out = 500)
  plot(h, freq = FALSE, col = "#bee4dc", border = "white", main = "",
    xlab = "Observed value", ylab = "Density", ylim = c(0, max(h$density, population_density(x, e)) * 1.15))
  lines(x, population_density(x, e), col = "#153c50", lwd = 2.5)
  rug(e$sample, col = "#658e88"); abline(v = e$mu, col = "#d96342", lwd = 2, lty = 2)
  legend("topright", c("Population density", "True mean"), col = c("#153c50", "#d96342"), lty = c(1, 2), lwd = 2, bty = "n", cex = .85)
}
plot_means <- function(e) {
  plot_style()
  h <- hist(e$intervals$mean, breaks = "FD", plot = FALSE)
  x <- seq(min(h$breaks), max(h$breaks), length.out = 500)
  reference <- dnorm(x, e$mu, e$sigma / sqrt(e$n))
  plot(h, freq = FALSE, col = "#bee4dc", border = "white", main = "",
    xlab = "Mean of a repeated sample", ylab = "Density", ylim = c(0, max(h$density, reference) * 1.15))
  lines(x, reference, col = "#153c50", lwd = 2.5, lty = 2); abline(v = e$mu, col = "#d96342", lwd = 2)
  legend("topright", "Normal reference: SD / sqrt(n)", col = "#153c50", lty = 2, lwd = 2, bty = "n", cex = .85)
}
plot_intervals <- function(e) {
  plot_style(); d <- head(e$intervals, 60)
  colors <- ifelse(d$contains_true_mean, "#168775", "#d96342")
  plot(range(d$lower, d$upper, e$mu), c(1, nrow(d)), type = "n", main = "",
    xlab = "Estimated mean and confidence interval", ylab = "Repeated sample")
  abline(v = e$mu, col = "#153c50", lwd = 2, lty = 2)
  segments(d$lower, d$repetition, d$upper, d$repetition, col = colors, lwd = 2)
  points(d$mean, d$repetition, pch = 16, col = colors, cex = .55)
}

ui <- fluidPage(title = "Sampling Lab | ShinyHarbor",
  tags$head(tags$style(HTML("
    :root{--ink:#163848;--muted:#5c727d;--teal:#087d70;--line:#dbe7e5}
    body{background:#f3f7f6;color:var(--ink);font-family:system-ui,-apple-system,'Segoe UI',sans-serif;font-size:15px}
    .container-fluid{max-width:1450px;margin:auto;padding:0 30px 35px}.mast{display:flex;justify-content:space-between;align-items:center;padding:23px 0;border-bottom:1px solid var(--line);gap:15px}
    .wordmark{font-weight:750;letter-spacing:-.5px;font-size:22px}.wordmark span{color:var(--teal)}.badge-live{font-size:12px;border:1px solid #b1d9cf;border-radius:30px;padding:7px 12px;background:#e2f2ec;color:#176b58}
    .hero{padding:30px 0}.eyebrow{text-transform:uppercase;letter-spacing:2px;font-size:11px;font-weight:750;color:var(--teal)}h1{font-size:clamp(32px,4vw,49px);letter-spacing:-1.8px;margin:8px 0 12px;font-weight:750}h2{font-size:19px;font-weight:700;margin:0 0 15px}.lead-copy{max-width:780px;color:var(--muted);line-height:1.65;margin:0}
    .lab-layout{display:grid;grid-template-columns:280px minmax(0,1fr);gap:22px;align-items:start}.lab-layout>*{min-width:0}.shiny-plot-output img{max-width:100%}.panel-card{background:white;border:1px solid var(--line);border-radius:17px;padding:23px;box-shadow:0 4px 20px #16384804}
    .controls .shiny-input-container{width:100%}.controls label{font-size:12px;color:#49636f;margin-bottom:8px}.controls .form-group{margin-bottom:18px}.controls .form-control{border-color:var(--line);box-shadow:none;border-radius:7px}.btn{border-radius:8px;font-size:13px;padding:10px 13px;border-color:#c8d9d5}.btn-primary{background:var(--teal);border-color:var(--teal);color:white;font-weight:650}.btn-primary:hover{background:#065f57;border-color:#065f57}#run{width:100%;padding:12px}
    .small-note{font-size:12px;color:var(--muted);line-height:1.65;margin:12px 0 0}.pending{color:#94602d;min-height:20px;font-size:12px;margin:10px 0}
    .metrics{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:13px;margin-bottom:20px}.metric{padding:18px 16px}.metric-label{font-size:11px;text-transform:uppercase;letter-spacing:.8px;color:var(--muted)}.metric-value{font-size:25px;font-weight:700;letter-spacing:-1px;margin:8px 0}.metric-note{font-size:11px;color:var(--muted);line-height:1.5}.metric:last-child{background:#153f4a;color:white}.metric:last-child .metric-label,.metric:last-child .metric-note{color:#c0d8d7}
    .nav-tabs{border-bottom:1px solid var(--line);margin-bottom:12px}.nav-tabs>li>a{color:var(--muted);font-size:13px;border:0;padding:12px}.nav-tabs>li.active>a,.nav-tabs>li.active>a:hover,.nav-tabs>li.active>a:focus{color:var(--teal);border:0;border-bottom:3px solid var(--teal);font-weight:700}
    .plot-copy{color:var(--muted);font-size:13px;line-height:1.7;margin:12px 6px}.interpretation{border-left:3px solid var(--teal);padding:2px 0 2px 16px;margin:14px 0;color:#355660;line-height:1.7;font-size:13px}.bottom-grid{display:grid;grid-template-columns:1.2fr 1fr;gap:20px;margin-top:20px}.download-row{display:flex;flex-wrap:wrap;gap:8px;margin-top:16px}details{font-size:13px;line-height:1.7}summary{cursor:pointer;font-weight:650;color:var(--teal)}footer{display:flex;justify-content:space-between;gap:20px;color:var(--muted);font-size:11px;margin:24px 0}.scenario{font-size:12px;color:var(--muted);margin-bottom:16px}a{color:var(--teal)}button:focus-visible,a:focus-visible{outline:3px solid #efae62;outline-offset:2px}
    @media(max-width:1050px){.metrics{grid-template-columns:repeat(2,1fr)}.bottom-grid{grid-template-columns:1fr}}@media(max-width:720px){.container-fluid{padding:0 15px 25px}.lab-layout{grid-template-columns:1fr}.panel-card{padding:17px}.nav-tabs>li>a{padding:9px;font-size:12px}footer{display:block}}
  ")), tags$script(HTML("$(document).on('shiny:connected',function(){$('#connection').text('Live connection');});$(document).on('shiny:disconnected',function(){$('#connection').text('Reconnecting...');});"))),
  tags$div(class = "mast", tags$div(class = "wordmark", "Shiny", tags$span("Harbor")), tags$span(id = "connection", class = "badge-live", role = "status", "Connecting...")),
  tags$div(class = "hero", tags$div(class = "eyebrow", "An interactive statistics playground"), h1("One sample. Many possible stories."), p(class = "lead-copy", "Meet Sampling Lab. Draw from a population you control, then repeat the experiment to see how much an estimate can move. Discover when a confidence interval earns its name.")),
  tags$div(class = "lab-layout",
    tags$aside(class = "panel-card controls", h2("Design your experiment"),
      selectInput("shape", "Population shape", c("Bell-shaped / normal" = "normal", "Right-skewed / exponential" = "skewed", "Heavy-tailed / Student t" = "heavy")),
      sliderInput("n", "Observations per sample", min = 5, max = 200, value = 30, step = 5),
      numericInput("mu", "True population mean", 0, min = -100, max = 100, step = .5),
      numericInput("sigma", "Population standard deviation", 1, min = .1, max = 20, step = .1),
      selectInput("level", "Confidence level", c("90%" = .90, "95%" = .95, "99%" = .99), selected = .95),
      selectInput("repetitions", "Repeated samples", c("500 (quick)" = 500, "2,000 (recommended)" = 2000, "5,000 (more precise)" = 5000), selected = 2000),
      numericInput("seed", "Random seed (for reproducibility)", 2026, min = 1, max = 10000000, step = 1),
      actionButton("new_seed", "Draw a new experiment", class = "btn-primary"),
      p(class = "small-note", "Every control updates the experiment automatically. The same settings and seed reproduce the same results. Draw a new experiment to change the seed.")),
    tags$main(tags$div(class = "scenario", textOutput("scenario")), uiOutput("metrics"),
      tags$section(class = "panel-card", tabsetPanel(id = "view",
        tabPanel("Your sample", plotOutput("sample_plot", height = "330px"), p(class = "plot-copy", "The bars describe one random sample. The dark curve is the population it came from; the dashed orange line marks its true mean. Even a fair sample will not look exactly like its population.")),
        tabPanel("Sampling distribution", plotOutput("means_plot", height = "330px"), p(class = "plot-copy", "Each mean comes from a fresh, independent sample of the same size. The dashed curve is a normal reference with standard deviation sigma / sqrt(n). Larger samples usually give more precise estimates; skewness and heavy tails can slow the approach to normality.")),
        tabPanel("Interval coverage", plotOutput("intervals_plot", height = "330px"), p(class = "plot-copy", "The first 60 intervals are shown, without sorting or selecting them. Green intervals contain the fixed true mean; orange intervals miss it. Coverage above uses every repeated sample, not only these 60."))),
        tags$div(class = "interpretation", textOutput("interpretation"))),
      tags$div(class = "bottom-grid", tags$section(class = "panel-card", h2("Take the experiment with you"), p(class = "small-note", "Download observations, every simulated interval, or a three-page plot report. Downloads use the last completed experiment."), tags$div(class = "download-row", downloadButton("sample_csv", "Sample CSV"), downloadButton("intervals_csv", "Intervals CSV"), downloadButton("report_pdf", "Plot report PDF"))),
        tags$section(class = "panel-card", h2("What the interval means"), p(class = "small-note", "A 95% method aims to include the fixed true mean in 95% of repeated samples. It does not assign a 95% probability to the true mean after you calculate one interval."),
          tags$details(tags$summary("Assumptions and simulation precision"), p("Intervals use the sample mean plus or minus a Student t critical value times the estimated standard error. Coverage is exact for independent normal observations; it is only approximate for the other populations. The heavy-tailed population is a t distribution with 3 degrees of freedom, rescaled to the chosen SD. The skewed population is a shifted, rescaled exponential."), p("Interval widths use the sample SD. The true SD is used only for the normal reference curve. Monte Carlo uncertainty describes the precision of the coverage simulation, not uncertainty about the population mean."))))
    )),
  tags$footer(tags$span("Sampling Lab / ShinyHarbor example"), textOutput("runtime", inline = TRUE)),
  tags$details(tags$summary("Explore what ShinyHarbor is serving"), p("Each browser session has its own experiment. Try two windows to compare them. Live updates exercise the Shiny connection; CSV and PDF buttons exercise file downloads. Watch app connections and traffic in the private usage dashboard. HTTPS, workers and access rules are configured in server administration; this public example cannot change server settings."))
)

server <- function(input, output, session) {
  settings <- reactive({
    req(input$shape, input$n, !is.null(input$mu), input$sigma, input$level, input$repetitions, input$seed)
    validate(need(is.finite(input$mu) && abs(input$mu) <= 100, "Choose a mean between -100 and 100."),
      need(is.finite(input$sigma) && input$sigma >= .1 && input$sigma <= 20, "Choose an SD between 0.1 and 20."),
      need(is.finite(input$seed) && input$seed >= 1 && input$seed <= 10000000 && input$seed == floor(input$seed), "Choose a whole-number seed from 1 to 10,000,000."))
    list(shape = input$shape, n = as.integer(input$n), mu = input$mu, sigma = input$sigma,
      level = as.numeric(input$level), repetitions = as.integer(input$repetitions), seed = as.integer(input$seed))
  })
  experiment <- reactive(do.call(simulate_experiment, settings()))
  observeEvent(input$new_seed, updateNumericInput(session, "seed", value = (isolate(input$seed) %% 10000000) + 1))
  output$scenario <- renderText({ e <- experiment(); sprintf("%s population  /  n = %d  /  %s repeated samples  /  seed %d", shape_name(e$shape), e$n, format(e$repetitions, big.mark = ","), e$seed) })
  output$metrics <- renderUI({
    e <- experiment()
    metric <- function(label, value, note) tags$article(class = "panel-card metric", tags$div(class = "metric-label", label), tags$div(class = "metric-value", value), tags$div(class = "metric-note", note))
    tags$div(class = "metrics", metric("Your sample mean", sprintf("%.2f", e$mean), sprintf("True mean: %.2f", e$mu)),
      metric("Estimated standard error", sprintf("%.3f", e$se), "Sample SD divided by sqrt(n)"),
      metric(sprintf("%.0f%% interval", 100 * e$level), sprintf("%.2f to %.2f", e$interval[1], e$interval[2]), "From this one sample"),
      metric("Simulated coverage", sprintf("%.1f%%", 100 * e$coverage), sprintf("Monte Carlo SE: %.2f percentage points", 100 * e$coverage_mcse)))
  })
  output$sample_plot <- renderPlot(plot_sample(experiment()), res = 110)
  output$means_plot <- renderPlot(plot_means(experiment()), res = 110)
  output$intervals_plot <- renderPlot(plot_intervals(experiment()), res = 110)
  output$interpretation <- renderText({ e <- experiment(); includes <- if (e$interval[1] <= e$mu && e$interval[2] >= e$mu) "includes" else "misses"
    sprintf("This sample's interval %s the true mean. Across %s independent samples, %.1f%% of intervals contained it; the nominal target is %.0f%%. Try a small sample from the right-skewed population, then increase n and compare coverage.", includes, format(e$repetitions, big.mark = ","), 100 * e$coverage, 100 * e$level) })
  output$sample_csv <- downloadHandler(function() paste0("sample-seed-", experiment()$seed, ".csv"), function(file) {
    e <- experiment(); write.csv(data.frame(observation = seq_along(e$sample), value = e$sample, population = e$shape, true_mean = e$mu, true_sd = e$sigma, seed = e$seed), file, row.names = FALSE)
  })
  output$intervals_csv <- downloadHandler(function() paste0("intervals-seed-", experiment()$seed, ".csv"), function(file) {
    e <- experiment(); write.csv(cbind(e$intervals, n = e$n, confidence = e$level, population = e$shape, true_mean = e$mu, true_sd = e$sigma, seed = e$seed), file, row.names = FALSE)
  })
  output$report_pdf <- downloadHandler("sampling-lab.pdf", function(file) {
    e <- experiment(); pdf(file, width = 9, height = 6, title = "Sampling Lab"); on.exit(dev.off())
    for (draw in list(plot_sample, plot_means, plot_intervals)) { draw(e); title(main = sprintf("Sampling Lab | %s | n=%d | seed=%d", shape_name(e$shape), e$n, e$seed), cex.main = .9) }
  }, contentType = "application/pdf")
  output$runtime <- renderText(paste("Base R", getRversion(), "+ Shiny", packageVersion("shiny"), "| No additional R packages"))
}
shinyApp(ui, server)
