library(shiny)
ui <- function(request) fluidPage(
  h1("Bookmark migration fixture"),
  numericInput("n", "Number", 2), textOutput("double"),
  fileInput("upload", "File to preserve"), textOutput("uploaded"),
  textOutput("upload_name"), textOutput("custom_state"), textOutput("sidecar"),
  textInput("after_restore", "Restore callback result", "new session"),
  actionButton("save", "Save bookmark"), textOutput("bookmark_url")
)
server <- function(input, output, session) {
  output$double <- renderText(input$n * 2)
  output$uploaded <- renderText({
    req(input$upload)
    paste(readLines(input$upload$datapath, encoding = "UTF-8", warn = FALSE), collapse = "\n")
  })
  output$upload_name <- renderText({ req(input$upload); input$upload$name })
  restored <- reactiveValues(custom = "new session", sidecar = "new session")
  onBookmark(function(state) {
    state$values$custom <- list(version = 1L, result = input$n * 13)
    writeLines(paste("sidecar", input$n), file.path(state$dir, "custom.txt"))
  })
  onRestore(function(state) {
    stopifnot(identical(state$values$custom$version, 1L))
    restored$custom <- as.character(state$values$custom$result)
    restored$sidecar <- readLines(file.path(state$dir, "custom.txt"), warn = FALSE)
  })
  onRestored(function(state) {
    updateTextInput(session, "after_restore", value = paste("restored", state$values$custom$result))
  })
  output$custom_state <- renderText(restored$custom)
  output$sidecar <- renderText(restored$sidecar)
  saved <- reactiveVal("")
  observeEvent(input$save, session$doBookmark())
  onBookmarked(function(url) saved(url))
  setBookmarkExclude(c("save", "after_restore"))
  output$bookmark_url <- renderText(saved())
}
shinyApp(ui, server, enableBookmarking = "server")
