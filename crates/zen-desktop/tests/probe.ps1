param([string]$Out)
Add-Type -AssemblyName System.Windows.Forms
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Zen Probe Window'
$form.Width = 480; $form.Height = 300; $form.StartPosition = 'CenterScreen'; $form.TopMost = $true
$box = New-Object System.Windows.Forms.TextBox
$box.Multiline = $true; $box.AcceptsReturn = $true; $box.Dock = 'Fill'; $box.Name = 'campo'
$button = New-Object System.Windows.Forms.Button
$button.Text = 'Guardar'; $button.Dock = 'Bottom'
$button.Add_Click({ [IO.File]::WriteAllText($Out, $box.Text, (New-Object Text.UTF8Encoding($false))) })
$form.Controls.Add($box); $form.Controls.Add($button)
$form.Add_Shown({ $box.Focus() })
[void]$form.ShowDialog()
