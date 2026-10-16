using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Threading;
using Markdig;
using Markdig.Extensions.Tables;
using Markdig.Extensions.TaskLists;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using Markdig.Renderers.Html;
using WpfList = System.Windows.Documents.List;
using MdTable = Markdig.Extensions.Tables.Table;
using WpfTable = System.Windows.Documents.Table;

namespace Cloudroom.Desktop;

public sealed class MarkdownLinkEventArgs(string target, RoutedEvent routedEvent) : RoutedEventArgs(routedEvent)
{
    public string Target { get; } = target;
}

/// <summary>Selectable native Markdown. No browser, HTML execution or resource loading.</summary>
public sealed class MarkdownView : RichTextBox
{
    public static readonly DependencyProperty SourceTextProperty = DependencyProperty.Register(nameof(SourceText), typeof(string), typeof(MarkdownView), new PropertyMetadata("", Changed));
    public static readonly DependencyProperty CodeLanguageProperty = DependencyProperty.Register(nameof(CodeLanguage), typeof(string), typeof(MarkdownView), new PropertyMetadata("", Changed));
    public static readonly DependencyProperty ContextSessionProperty = DependencyProperty.Register(nameof(ContextSession), typeof(string), typeof(MarkdownView), new PropertyMetadata(""));
    public static readonly RoutedEvent LinkActivatedEvent = EventManager.RegisterRoutedEvent("LinkActivated", RoutingStrategy.Bubble, typeof(RoutedEventHandler), typeof(MarkdownView));
    public string SourceText { get => (string)GetValue(SourceTextProperty); set => SetValue(SourceTextProperty, value); }
    public string CodeLanguage { get => (string)GetValue(CodeLanguageProperty); set => SetValue(CodeLanguageProperty, value); }
    public string ContextSession { get => (string)GetValue(ContextSessionProperty); set => SetValue(ContextSessionProperty, value); }
    private static readonly MarkdownPipeline Pipeline = new MarkdownPipelineBuilder { MaximumNestingDepth = 32 }.DisableHtml().UseAutoIdentifiers(Markdig.Extensions.AutoIdentifiers.AutoIdentifierOptions.GitHub).UsePipeTables().UseTaskLists().UseAutoLinks().UseEmphasisExtras(Markdig.Extensions.EmphasisExtras.EmphasisExtraOptions.Strikethrough).Build();
    private static readonly Brush Ink = Brush("#EDF1F9"), Muted = Brush("#95A2B6"), Accent = Brush("#B5ACFF"), CodeBg = Brush("#10151D"), Green = Brush("#A9D4AC"), Blue = Brush("#92BBFF");
    private readonly DispatcherTimer renderTimer;
    private int budget;
    private readonly Dictionary<string, Paragraph> anchors = [];
    public MarkdownView()
    {
        IsReadOnly = true; IsDocumentEnabled = true; IsReadOnlyCaretVisible = false;
        Background = Brushes.Transparent; BorderThickness = new Thickness(0); Padding = new Thickness(0);
        HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled; VerticalScrollBarVisibility = ScrollBarVisibility.Disabled;
        renderTimer = new DispatcherTimer(DispatcherPriority.Background) { Interval = TimeSpan.FromMilliseconds(120) };
        renderTimer.Tick += (_, _) => { renderTimer.Stop(); RenderNow(); };
        Unloaded += (_, _) => renderTimer.Stop(); Loaded += (_, _) => RenderNow();
    }
    private static Brush Brush(string color) { var brush = new SolidColorBrush((Color)ColorConverter.ConvertFromString(color)); brush.Freeze(); return brush; }
    private static void Changed(DependencyObject value, DependencyPropertyChangedEventArgs e)
    {
        var view = (MarkdownView)value; view.renderTimer.Stop(); if (view.IsLoaded) view.renderTimer.Start();
    }
    public void RenderNow()
    {
        renderTimer.Stop(); budget = 10000; anchors.Clear();
        var document = new FlowDocument { PagePadding = new Thickness(0), FontFamily = new FontFamily("Segoe UI"), FontSize = 14, Foreground = Ink, Background = Brushes.Transparent, ColumnWidth = double.PositiveInfinity };
        string source = SourceText ?? "";
        if (source.Length > 96000) source = source[..96000] + "\n[Display limit reached; use Copy source for the original text.]";
        try
        {
            if (CodeLanguage.Length > 0) document.Blocks.Add(Code(source, CodeLanguage));
            else foreach (var block in Markdown.Parse(source, Pipeline)) AddBlock(document.Blocks, block, 0);
        }
        catch (Exception error) when (error is InvalidOperationException or ArgumentException)
        {
            document.Blocks.Clear(); document.Blocks.Add(new Paragraph(new Run(source)));
        }
        if (budget <= 0) document.Blocks.Add(new Paragraph(new Run("[Rendering limit reached. Copy source retains the original text.]")) { Foreground = Muted });
        Document = document;
    }
    public bool ScrollToAnchor(string anchor)
    {
        if (!anchors.TryGetValue(Uri.UnescapeDataString(anchor), out var paragraph)) return false;
        paragraph.BringIntoView(); return true;
    }
    public bool SelectSourceLine(int line)
    {
        string text = SourceText ?? "";
        if (text.Length > 96000) text = text[..96000];
        int offset = 0;
        for (int i = 1; i < line; i++) { int next = text.IndexOf('\n', offset); if (next < 0) return false; offset = next + 1; }
        var run = new Run(text);
        Document = new FlowDocument(new Paragraph(run)) { PagePadding = new Thickness(0), FontFamily = new FontFamily("Consolas"), FontSize = 12, Foreground = Ink };
        var pointer = run.ContentStart.GetPositionAtOffset(offset);
        if (pointer == null) return false;
        Selection.Select(pointer, pointer); Focus(); UpdateLayout();
        ScrollToVerticalOffset(Math.Max(0, VerticalOffset + pointer.GetCharacterRect(LogicalDirection.Forward).Top - 24));
        return true;
    }
    private void AddBlock(BlockCollection output, Markdig.Syntax.Block block, int depth)
    {
        if (budget-- <= 0 || depth > 32) return;
        switch (block)
        {
            case HeadingBlock heading:
                var title = Paragraph(heading.Inline); title.FontWeight = FontWeights.SemiBold; title.FontSize = heading.Level switch { 1 => 25, 2 => 21, 3 => 18, _ => 15 }; title.Margin = new Thickness(0, 12, 0, 10); output.Add(title); if (heading.GetAttributes().Id is string id) anchors[id] = title; break;
            case FencedCodeBlock code: output.Add(Code(code.Lines.ToString(), code.Info ?? "text")); break;
            case CodeBlock code: output.Add(Code(code.Lines.ToString(), "text")); break;
            case ParagraphBlock paragraph: output.Add(Paragraph(paragraph.Inline)); break;
            case QuoteBlock quote:
                var section = new Section { BorderBrush = Accent, BorderThickness = new Thickness(3, 0, 0, 0), Padding = new Thickness(12, 4, 0, 4), Margin = new Thickness(0, 8, 0, 12), Foreground = Muted };
                foreach (var child in quote) AddBlock(section.Blocks, child, depth + 1); output.Add(section); break;
            case ListBlock list:
                var native = new WpfList { MarkerStyle = list.IsOrdered ? TextMarkerStyle.Decimal : TextMarkerStyle.Disc, Margin = new Thickness(0, 4, 0, 12), Padding = new Thickness(24, 0, 0, 0) };
                if (int.TryParse(list.OrderedStart, out int start) && start > 0) native.StartIndex = start;
                foreach (var child in list.OfType<ListItemBlock>()) { if (budget <= 0) break; var item = new ListItem(); foreach (var nested in child) AddBlock(item.Blocks, nested, depth + 1); if (item.Blocks.Count == 0) item.Blocks.Add(new Paragraph()); native.ListItems.Add(item); }
                output.Add(native); break;
            case MdTable table:
                var grid = new WpfTable { CellSpacing = 0, Margin = new Thickness(0, 8, 0, 14) }; var group = new TableRowGroup(); grid.RowGroups.Add(group);
                foreach (var row in table.OfType<Markdig.Extensions.Tables.TableRow>())
                {
                    if (budget-- <= 0) break;
                    var nativeRow = new System.Windows.Documents.TableRow(); group.Rows.Add(nativeRow);
                    foreach (var cell in row.OfType<Markdig.Extensions.Tables.TableCell>())
                    {
                        if (budget-- <= 0) break;
                        var nativeCell = new System.Windows.Documents.TableCell { BorderBrush = Brush("#414C60"), BorderThickness = new Thickness(.5), Padding = new Thickness(9, 7, 9, 7), FontWeight = row.IsHeader ? FontWeights.SemiBold : FontWeights.Normal };
                        foreach (var nested in cell) AddBlock(nativeCell.Blocks, nested, depth + 1); nativeRow.Cells.Add(nativeCell);
                    }
                }
                output.Add(grid); break;
            case ThematicBreakBlock: output.Add(new Paragraph(new Run("────────────────────────")) { Foreground = Muted }); break;
            case ContainerBlock container: foreach (var child in container) AddBlock(output, child, depth + 1); break;
            case LeafBlock leaf: output.Add(new Paragraph(new Run(leaf.Lines.ToString()))); break;
        }
    }
    private Paragraph Paragraph(ContainerInline? source)
    {
        var paragraph = new Paragraph { Margin = new Thickness(0, 0, 0, 12), LineHeight = 22 };
        if (source != null) AddInline(paragraph.Inlines, source, 0);
        return paragraph;
    }
    private void AddInline(InlineCollection output, Markdig.Syntax.Inlines.Inline source, int depth)
    {
        if (budget-- <= 0 || depth > 32) return;
        switch (source)
        {
            case LiteralInline literal: output.Add(new Run(literal.Content.ToString())); break;
            case CodeInline code: output.Add(new Run(code.Content) { FontFamily = new FontFamily("Consolas"), Background = CodeBg, Foreground = Green }); break;
            case LineBreakInline line: output.Add(line.IsHard ? new LineBreak() : new Run(" ")); break;
            case EmphasisInline emphasis:
                var span = new Span(); if (emphasis.DelimiterChar == '~') span.TextDecorations = TextDecorations.Strikethrough; else if (emphasis.DelimiterCount >= 2) span.FontWeight = FontWeights.Bold; else span.FontStyle = FontStyles.Italic;
                foreach (var child in emphasis) AddInline(span.Inlines, child, depth + 1); output.Add(span); break;
            case LinkInline link:
                if (link.IsImage) { output.Add(new Run("[Image: " + (link.Title ?? link.Url ?? "attachment") + "]") { Foreground = Muted }); break; }
                AddLink(output, link.Url ?? "", target => { foreach (var child in link) AddInline(target, child, depth + 1); }); break;
            case AutolinkInline link: AddLink(output, link.Url, target => target.Add(new Run(link.Url))); break;
            case TaskList task: output.Add(new Run(task.Checked ? "☑ " : "☐ ") { Foreground = Accent }); break;
            case ContainerInline container: foreach (var child in container) AddInline(output, child, depth + 1); break;
            default: output.Add(new Run(source.ToString() ?? "")); break;
        }
    }
    public static bool AllowedLink(string target)
    {
        if (target.Length == 0 || target.Length > 4096 || target.Any(char.IsControl)) return false;
        if (Regex.IsMatch(target, @"^(?:\\\\\?\\)?[a-zA-Z]:[\\/]")) return true;
        if (Uri.TryCreate(target, UriKind.Absolute, out var uri)) return uri.Scheme is "http" or "https" or "file" && uri.UserInfo.Length == 0 && !uri.IsUnc;
        return !target.StartsWith("//", StringComparison.Ordinal) && !target.StartsWith("\\\\", StringComparison.Ordinal) && !target.Split('/')[0].Contains(':');
    }
    private void AddLink(InlineCollection output, string target, Action<InlineCollection> content)
    {
        if (!AllowedLink(target)) { var text = new Span(); content(text.Inlines); output.Add(text); return; }
        var link = new Hyperlink { Foreground = Accent, ToolTip = target };
        content(link.Inlines); if (link.Inlines.Count == 0) link.Inlines.Add(new Run(target));
        link.Click += (_, e) => { e.Handled = true; if (target.StartsWith('#') && ScrollToAnchor(target[1..])) return; RaiseEvent(new MarkdownLinkEventArgs(target, LinkActivatedEvent)); };
        output.Add(link);
    }
    private Paragraph Code(string text, string language)
    {
        var paragraph = new Paragraph { FontFamily = new FontFamily("Consolas"), FontSize = 12, LineHeight = 18, Background = CodeBg, Padding = new Thickness(12), Margin = new Thickness(0, 4, 0, 14) };
        paragraph.Inlines.Add(new Run(language.Split(' ')[0].ToLowerInvariant() + "\n") { Foreground = Muted });
        bool supported = language.Split(' ')[0].ToLowerInvariant() is "rust" or "rs" or "csharp" or "cs" or "python" or "py" or "javascript" or "js" or "typescript" or "ts" or "powershell" or "ps1" or "json";
        if (!supported) { paragraph.Inlines.Add(new Run(text)); return paragraph; }
        const string pattern = "(?<comment>//[^\\r\\n]*|#[^\\r\\n]*|/\\*[\\s\\S]*?\\*/)|(?<str>\"(?:\\\\.|[^\"\\\\])*\"|'(?:\\\\.|[^'\\\\])*')|(?<key>\\b(?:fn|let|mut|pub|use|impl|struct|enum|return|if|else|for|while|match|class|public|private|static|void|var|new|using|async|await|def|import|from|print|const|function|export|true|false|null|None|True|False)\\b)|(?<number>\\b\\d+(?:\\.\\d+)?\\b)";
        try
        {
            int offset = 0;
            foreach (Match match in Regex.Matches(text, pattern, RegexOptions.None, TimeSpan.FromMilliseconds(80)))
            {
                if (budget-- <= 0) break;
                if (match.Index > offset) paragraph.Inlines.Add(new Run(text[offset..match.Index]));
                paragraph.Inlines.Add(new Run(match.Value) { Foreground = match.Groups["comment"].Success ? Muted : match.Groups["str"].Success ? Green : match.Groups["key"].Success ? Accent : Blue }); offset = match.Index + match.Length;
            }
            if (offset < text.Length) paragraph.Inlines.Add(new Run(text[offset..]));
        }
        catch (RegexMatchTimeoutException) { paragraph.Inlines.Clear(); paragraph.Inlines.Add(new Run(text)); }
        return paragraph;
    }
}
