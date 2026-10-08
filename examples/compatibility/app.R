library(shiny)
ui <- function(request) fluidPage(
  titlePanel("ShinyHarbor browser fixture"),
  numericInput("n", "Value", 2), textOutput("double"),
  textInput("text", "Unicode text", "Zażółć gęślą jaźń 日本語"), textOutput("echo"),
  fileInput("upload", "Upload text"), textOutput("uploaded"),
  downloadButton("download", "Download CSV"),
  actionButton("dynamic", "Add UI"), uiOutput("dynamic_ui"),
  bookmarkButton(), textOutput("pid")
)
server <- function(input, output, session) {
  output$double <- renderText(as.numeric(input$n) * 2)
  output$echo <- renderText(input$text)
  output$uploaded <- renderText({ req(input$upload); paste(readLines(input$upload$datapath, warn = FALSE), collapse = "\n") })
  output$download <- downloadHandler(filename = function() "hello world.csv", content = function(file) {
    write.csv(data.frame(value = input$n, text = input$text), file, row.names = FALSE, fileEncoding = "UTF-8")
  })
  output$dynamic_ui <- renderUI({ req(input$dynamic); tags$strong(id = "added", "Dynamic UI is working") })
  output$pid <- renderText(paste("Worker PID:", Sys.getpid()))
}
shinyApp(ui, server, enableBookmarking = "url")
