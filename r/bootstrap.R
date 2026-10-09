# Original ShinyHarbor launcher, not Posit's Shiny Server launcher.
# The application is evaluated by the installed R packages, not translated.
local({
  # A Unix LC_* value inherited by a Windows service can make R fall back to
  # ASCII. Modern Windows R uses UCRT and supports this UTF-8 locale spelling.
  if (.Platform$OS.type == "windows" && !isTRUE(l10n_info()[["UTF-8"]])) {
    Sys.setlocale("LC_CTYPE", ".UTF-8")
  }
  app <- Sys.getenv("HARBOR_APP", unset = NA_character_)
  ready_file <- Sys.getenv("HARBOR_READY", unset = NA_character_)
  if (is.na(app) || is.na(ready_file)) stop("Missing launcher environment")
  if (!requireNamespace("shiny", quietly = TRUE)) {
    stop("Install shiny into the R library of the service account")
  }
  runtime <- Sys.getenv("HARBOR_RUNTIME", unset = NA_character_)
  if (is.na(runtime)) stop("Missing runtime integration file")
  source(runtime, local = TRUE)
  app <- harbor_path(app)
  socket <- Sys.getenv("HARBOR_SOCKET")
  port <- if (nzchar(socket)) structure(socket, mask = strtoi("077", 8L)) else httpuv::randomPort()
  # This is the hosted-runtime protocol target, not a claim of conformance.
  # serverInfo() identifies the actual implementation and package version.
  Sys.setenv(SHINY_PORT = as.character(port), SHINY_SERVER_VERSION = "1.5.23")
  shiny:::setServerInfo(shinyServer = TRUE, edition = "ShinyHarbor",
    version = Sys.getenv("HARBOR_SERVER_VERSION", "development"),
    protocolVersion = "1.5.23")
  harbor_configure(app, Sys.getenv("HARBOR_BOOKMARKS"),
    Sys.getenv("HARBOR_SHARED_SECRET"),
    sanitize = identical(Sys.getenv("HARBOR_SANITIZE_ERRORS", "true"), "true"))
  announce <- function(url) {
    # Same-directory rename: readers never see a partially-written URL.
    temp <- paste0(ready_file, ".tmp")
    writeLines(url, temp, useBytes = TRUE)
    if (!file.rename(temp, ready_file)) stop("Cannot publish readiness file")
  }
  if (grepl("\\.rmd$", app, ignore.case = TRUE)) {
    if (!requireNamespace("rmarkdown", quietly = TRUE)) stop("Install rmarkdown")
    rmarkdown::run(file = app, auto_reload = FALSE,
      shiny_args = list(host = "127.0.0.1", port = port, launch.browser = announce))
  } else {
    shiny::runApp(app, host = "127.0.0.1", port = port,
      launch.browser = announce, quiet = FALSE)
  }
})
